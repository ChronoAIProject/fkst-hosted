//! Filesystem boundary for the local SourceWorkspaceManager. Host I/O uses pinned
//! directories, never a re-resolved absolute child path. Path-only providers must
//! still protect their own I/O: boundary identity checks cannot close their races.

#[cfg(unix)]
mod platform {
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path, PathBuf};
    use std::sync::Arc;

    use nix::dir::Dir;
    use nix::errno::Errno;
    use nix::fcntl::{open, openat, AtFlags, OFlag};
    use nix::sys::stat::{fchmod, fstat, fstatat, mkdirat, FileStat, Mode, SFlag};
    use nix::unistd::{unlinkat, UnlinkatFlags};

    use crate::RunError;

    pub(crate) struct Directory {
        file: File,
        parent: Option<Arc<Directory>>,
        path: PathBuf,
    }

    pub(crate) struct PinnedFile {
        file: File,
        parent: Arc<Directory>,
        name: OsString,
    }

    // At most 32 root components plus 32 subtree levels are pinned per walk.
    // Width consumes bounded snapshot memory, not one descriptor per entry.
    const MAX_ROOT_COMPONENTS: usize = 32;
    const MAX_TREE_DEPTH: usize = 32;
    const MAX_TREE_ENTRIES: usize = 10_000;

    pub(crate) struct Tree {
        directory: Arc<Directory>,
        snapshot: Node,
    }

    struct Entry {
        name: OsString,
        identity: FileStat,
    }

    struct Node {
        files: Vec<Entry>,
        children: Vec<(Entry, Node)>,
    }

    fn budget_exceeded() -> RunError {
        RunError::Lifecycle(
            "workspace traversal exceeds root depth 32, subtree depth 32, or 10000 entries",
        )
    }

    fn io(error: Errno) -> RunError {
        std::io::Error::from_raw_os_error(error as i32).into()
    }

    fn changed() -> RunError {
        RunError::Lifecycle("lifecycle filesystem identity changed or is unsupported")
    }

    fn same(left: &FileStat, right: &FileStat) -> bool {
        left.st_dev == right.st_dev
            && left.st_ino == right.st_ino
            && SFlag::from_bits_truncate(left.st_mode) == SFlag::from_bits_truncate(right.st_mode)
    }

    fn kind(stat: &FileStat) -> SFlag {
        SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT
    }

    fn component(name: &OsStr) -> Result<(), RunError> {
        let mut parts = Path::new(name).components();
        if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
            return Err(changed());
        }
        Ok(())
    }

    fn stat(directory: &Directory, name: &OsStr) -> Result<Option<FileStat>, RunError> {
        component(name)?;
        match fstatat(&directory.file, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(stat) => Ok(Some(stat)),
            Err(Errno::ENOENT) => Ok(None),
            Err(error) => Err(io(error)),
        }
    }

    fn directory_flags() -> OFlag {
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC
    }

    impl Directory {
        pub(crate) fn prepare(path: &Path) -> Result<Arc<Self>, RunError> {
            if !path.is_absolute()
                || path == Path::new("/")
                || path
                    .components()
                    .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            {
                return Err(RunError::Lifecycle(
                    "lifecycle roots must be absolute confined directories",
                ));
            }
            if path
                .components()
                .filter(|part| matches!(part, Component::Normal(_)))
                .count()
                > MAX_ROOT_COMPONENTS
            {
                return Err(budget_exceeded());
            }
            let mut directory = Arc::new(Self {
                file: open("/", directory_flags(), Mode::empty())
                    .map_err(io)?
                    .into(),
                parent: None,
                path: PathBuf::from("/"),
            });
            for part in path.components() {
                if let Component::Normal(name) = part {
                    directory = directory.child(name, true)?.ok_or_else(changed)?;
                }
            }
            Ok(directory)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }

        pub(crate) fn ensure_attached(&self) -> Result<(), RunError> {
            if let Some(parent) = &self.parent {
                parent.ensure_attached()?;
                let current = stat(parent, self.path.file_name().ok_or_else(changed)?)?
                    .ok_or_else(changed)?;
                if !same(&current, &fstat(&self.file).map_err(io)?)
                    || kind(&current) != SFlag::S_IFDIR
                {
                    return Err(changed());
                }
            }
            Ok(())
        }

        pub(crate) fn child(
            self: &Arc<Self>,
            name: &OsStr,
            create: bool,
        ) -> Result<Option<Arc<Self>>, RunError> {
            self.ensure_attached()?;
            let existing = stat(self, name)?;
            if existing.is_none() {
                if !create {
                    return Ok(None);
                }
                match mkdirat(&self.file, name, Mode::S_IRWXU) {
                    Ok(()) | Err(Errno::EEXIST) => (),
                    Err(error) => return Err(io(error)),
                }
            }
            let expected = stat(self, name)?.ok_or_else(changed)?;
            if kind(&expected) != SFlag::S_IFDIR {
                return Err(changed());
            }
            let file: File = openat(&self.file, name, directory_flags(), Mode::empty())
                .map_err(io)?
                .into();
            if !same(&expected, &fstat(&file).map_err(io)?) {
                return Err(changed());
            }
            let child = Arc::new(Self {
                file,
                parent: Some(self.clone()),
                path: self.path.join(name),
            });
            child.ensure_attached()?;
            Ok(Some(child))
        }

        pub(crate) fn create_child(self: &Arc<Self>, name: &OsStr) -> Result<Arc<Self>, RunError> {
            component(name)?;
            self.ensure_attached()?;
            mkdirat(&self.file, name, Mode::S_IRWXU).map_err(io)?;
            self.child(name, false)?.ok_or_else(changed)
        }

        pub(crate) fn open_file(
            self: &Arc<Self>,
            name: &OsStr,
        ) -> Result<Option<PinnedFile>, RunError> {
            self.ensure_attached()?;
            let Some(expected) = stat(self, name)? else {
                return Ok(None);
            };
            if kind(&expected) != SFlag::S_IFREG || expected.st_nlink != 1 {
                return Err(changed());
            }
            let file: File = openat(
                &self.file,
                name,
                OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(io)?
            .into();
            if !same(&expected, &fstat(&file).map_err(io)?) {
                return Err(changed());
            }
            let file = PinnedFile {
                file,
                parent: self.clone(),
                name: name.to_owned(),
            };
            file.ensure_attached()?;
            Ok(Some(file))
        }

        pub(crate) fn write_new(
            self: &Arc<Self>,
            name: &OsStr,
            bytes: &[u8],
            readonly: bool,
        ) -> Result<(), RunError> {
            component(name)?;
            self.ensure_attached()?;
            let mut file: File = openat(
                &self.file,
                name,
                OFlag::O_WRONLY
                    | OFlag::O_CREAT
                    | OFlag::O_EXCL
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_CLOEXEC,
                Mode::S_IRUSR | Mode::S_IWUSR,
            )
            .map_err(io)?
            .into();
            file.write_all(bytes)?;
            if readonly {
                fchmod(&file, Mode::S_IRUSR).map_err(io)?;
            }
            file.sync_all()?;
            let pinned = PinnedFile {
                file,
                parent: self.clone(),
                name: name.to_owned(),
            };
            pinned.ensure_attached()
        }

        pub(crate) fn tree(self: &Arc<Self>) -> Result<Tree, RunError> {
            self.tree_with_reserved_entries(0)
        }

        pub(crate) fn tree_with_reserved_entries(
            self: &Arc<Self>,
            reserved: usize,
        ) -> Result<Tree, RunError> {
            let mut remaining = MAX_TREE_ENTRIES
                .checked_sub(reserved)
                .ok_or_else(budget_exceeded)?;
            let snapshot = Node::scan(self, 0, &mut remaining)?;
            let tree = Tree {
                directory: self.clone(),
                snapshot,
            };
            tree.ensure_attached()?;
            Ok(tree)
        }
    }

    impl PinnedFile {
        pub(crate) fn ensure_attached(&self) -> Result<(), RunError> {
            self.parent.ensure_attached()?;
            let current = stat(&self.parent, &self.name)?.ok_or_else(changed)?;
            if kind(&current) != SFlag::S_IFREG
                || current.st_nlink != 1
                || !same(&current, &fstat(&self.file).map_err(io)?)
            {
                return Err(changed());
            }
            Ok(())
        }

        pub(crate) fn bytes(&self) -> Result<Vec<u8>, RunError> {
            self.ensure_attached()?;
            let mut bytes = vec![];
            let mut file = &self.file;
            file.seek(SeekFrom::Start(0))?;
            file.read_to_end(&mut bytes)?;
            self.ensure_attached()?;
            Ok(bytes)
        }
    }

    impl Entry {
        fn directory(&self, parent: &Arc<Directory>) -> Result<Arc<Directory>, RunError> {
            let directory = parent.child(&self.name, false)?.ok_or_else(changed)?;
            if !same(&self.identity, &fstat(&directory.file).map_err(io)?) {
                return Err(changed());
            }
            Ok(directory)
        }

        fn file(&self, parent: &Arc<Directory>) -> Result<PinnedFile, RunError> {
            let file = parent.open_file(&self.name)?.ok_or_else(changed)?;
            let current = fstat(&file.file).map_err(io)?;
            if !same(&self.identity, &current) || self.identity.st_nlink != current.st_nlink {
                return Err(changed());
            }
            Ok(file)
        }

        fn unlink_file(&self, parent: &Arc<Directory>) -> Result<(), RunError> {
            let file = self.file(parent)?;
            file.ensure_attached()?;
            // POSIX has no atomic compare-inode-and-unlink. A substituted leaf
            // is never followed, but its name can be unlinked inside this parent.
            unlinkat(
                &parent.file,
                self.name.as_os_str(),
                UnlinkatFlags::NoRemoveDir,
            )
            .map_err(io)
        }
    }

    impl Node {
        fn scan(
            directory: &Arc<Directory>,
            depth: usize,
            remaining: &mut usize,
        ) -> Result<Self, RunError> {
            directory.ensure_attached()?;
            let mut entries =
                Dir::openat(&directory.file, ".", directory_flags(), Mode::empty()).map_err(io)?;
            let mut names = Vec::new();
            for entry in entries.iter() {
                let entry = entry.map_err(io)?;
                let name = OsStr::from_bytes(entry.file_name().to_bytes());
                if name == OsStr::new(".") || name == OsStr::new("..") {
                    continue;
                }
                *remaining = remaining.checked_sub(1).ok_or_else(budget_exceeded)?;
                names.push(Entry {
                    name: name.to_owned(),
                    identity: stat(directory, name)?.ok_or_else(changed)?,
                });
            }
            // Close iteration before descending: one directory FD per depth,
            // rather than a directory plus iterator per depth or per sibling.
            drop(entries);
            let mut node = Self {
                files: Vec::new(),
                children: Vec::new(),
            };
            for entry in names {
                match kind(&entry.identity) {
                    SFlag::S_IFDIR => {
                        if depth == MAX_TREE_DEPTH {
                            return Err(budget_exceeded());
                        }
                        let child = entry.directory(directory)?;
                        let snapshot = Self::scan(&child, depth + 1, remaining)?;
                        node.children.push((entry, snapshot));
                    }
                    SFlag::S_IFREG => {
                        entry.file(directory)?.ensure_attached()?;
                        node.files.push(entry);
                    }
                    _ => return Err(changed()),
                }
            }
            directory.ensure_attached()?;
            Ok(node)
        }

        fn ensure_attached(&self, directory: &Arc<Directory>) -> Result<(), RunError> {
            directory.ensure_attached()?;
            for file in &self.files {
                file.file(directory)?.ensure_attached()?;
            }
            for (entry, child) in &self.children {
                child.ensure_attached(&entry.directory(directory)?)?;
            }
            Ok(())
        }

        fn remove_contents(
            &self,
            directory: &Arc<Directory>,
            retained: Option<&OsStr>,
        ) -> Result<(), RunError> {
            for (entry, child) in &self.children {
                let opened = entry.directory(directory)?;
                child.remove_contents(&opened, None)?;
                opened.ensure_attached()?;
                unlinkat(
                    &directory.file,
                    entry.name.as_os_str(),
                    UnlinkatFlags::RemoveDir,
                )
                .map_err(io)?;
            }
            for file in &self.files {
                if retained != Some(file.name.as_os_str()) {
                    file.unlink_file(directory)?;
                }
            }
            Ok(())
        }
    }

    impl Tree {
        pub(crate) fn ensure_attached(&self) -> Result<(), RunError> {
            self.snapshot.ensure_attached(&self.directory)
        }

        pub(crate) fn remove(self, marker_name: &OsStr) -> Result<(), RunError> {
            self.ensure_attached()?;
            let marker = self
                .snapshot
                .files
                .iter()
                .find(|entry| entry.name == marker_name)
                .ok_or_else(changed)?;
            self.snapshot
                .remove_contents(&self.directory, Some(marker_name))?;
            // New entries or any cleanup failure leave the original ownership
            // marker available for a later stop attempt.
            let remaining = self.directory.tree()?;
            if !remaining.snapshot.children.is_empty()
                || remaining.snapshot.files.len() != 1
                || remaining.snapshot.files[0].name != marker_name
            {
                return Err(changed());
            }
            marker.unlink_file(&self.directory)?;
            // Marker unlink and rmdir are separate syscalls; a crash or new entry
            // in this final window still needs future durable ownership recovery.
            self.directory.ensure_attached()?;
            let parent = self.directory.parent.as_ref().ok_or_else(changed)?;
            unlinkat(
                &parent.file,
                self.directory.path.file_name().ok_or_else(changed)?,
                UnlinkatFlags::RemoveDir,
            )
            .map_err(io)
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use crate::RunError;
    use std::ffi::OsStr;
    use std::path::Path;
    use std::sync::Arc;

    pub(crate) struct Directory;
    pub(crate) struct PinnedFile;
    pub(crate) struct Tree;
    fn unsupported<T>() -> Result<T, RunError> {
        Err(RunError::Lifecycle(
            "confined source workspaces are unsupported on this platform",
        ))
    }
    impl Directory {
        pub(crate) fn prepare(_: &Path) -> Result<Arc<Self>, RunError> {
            unsupported()
        }
        pub(crate) fn path(&self) -> &Path {
            Path::new("")
        }
        pub(crate) fn ensure_attached(&self) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn child(
            self: &Arc<Self>,
            _: &OsStr,
            _: bool,
        ) -> Result<Option<Arc<Self>>, RunError> {
            unsupported()
        }
        pub(crate) fn create_child(self: &Arc<Self>, _: &OsStr) -> Result<Arc<Self>, RunError> {
            unsupported()
        }
        pub(crate) fn open_file(
            self: &Arc<Self>,
            _: &OsStr,
        ) -> Result<Option<PinnedFile>, RunError> {
            unsupported()
        }
        pub(crate) fn write_new(
            self: &Arc<Self>,
            _: &OsStr,
            _: &[u8],
            _: bool,
        ) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn tree(self: &Arc<Self>) -> Result<Tree, RunError> {
            unsupported()
        }
        pub(crate) fn tree_with_reserved_entries(
            self: &Arc<Self>,
            _: usize,
        ) -> Result<Tree, RunError> {
            unsupported()
        }
    }
    impl PinnedFile {
        pub(crate) fn ensure_attached(&self) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn bytes(&self) -> Result<Vec<u8>, RunError> {
            unsupported()
        }
    }
    impl Tree {
        pub(crate) fn ensure_attached(&self) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn remove(self, _: &OsStr) -> Result<(), RunError> {
            unsupported()
        }
    }
}

pub(crate) use platform::{Directory, PinnedFile};

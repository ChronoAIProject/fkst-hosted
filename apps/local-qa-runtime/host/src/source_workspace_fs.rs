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
    use nix::fcntl::{open, openat, AtFlags, Flock, FlockArg, OFlag};
    use nix::sys::stat::{fchmod, fstat, fstatat, mkdirat, FileStat, Mode, SFlag};
    use nix::unistd::{linkat, unlinkat, UnlinkatFlags};

    use super::super::cache_checkpoint;

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

        #[cfg(feature = "local-bundle-provider")]
        pub(crate) fn open_existing(path: &Path) -> Result<Arc<Self>, RunError> {
            if !path.is_absolute()
                || path == Path::new("/")
                || path.components().count() > MAX_ROOT_COMPONENTS + 1
                || path
                    .components()
                    .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            {
                return Err(changed());
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
                    directory = directory.child(name, false)?.ok_or_else(changed)?;
                }
            }
            Ok(directory)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }

        pub(crate) fn overlaps(&self, other: &Self) -> Result<bool, RunError> {
            self.ensure_attached()?;
            other.ensure_attached()?;
            Ok(self.contains_identity(&fstat(&other.file).map_err(io)?)?
                || other.contains_identity(&fstat(&self.file).map_err(io)?)?)
        }

        fn contains_identity(&self, identity: &FileStat) -> Result<bool, RunError> {
            if same(&fstat(&self.file).map_err(io)?, identity) {
                return Ok(true);
            }
            self.parent
                .as_ref()
                .map(|parent| parent.contains_identity(identity))
                .unwrap_or(Ok(false))
        }

        pub(crate) fn identity(&self) -> Result<String, RunError> {
            self.ensure_attached()?;
            let stat = fstat(&self.file).map_err(io)?;
            let parent = self
                .parent
                .as_ref()
                .map(|parent| parent.identity())
                .transpose()?
                .unwrap_or_default();
            #[cfg(target_os = "macos")]
            let birth = format!("{}:{}", stat.st_birthtime, stat.st_birthtime_nsec);
            #[cfg(not(target_os = "macos"))]
            let birth = String::new();
            Ok(format!("{parent}/{}:{}:{birth}", stat.st_dev, stat.st_ino))
        }

        pub(crate) fn remove_empty(self: &Arc<Self>) -> Result<(), RunError> {
            let tree = self.tree()?;
            if !tree.snapshot.files.is_empty() || !tree.snapshot.children.is_empty() {
                return Err(changed());
            }
            self.ensure_attached()?;
            let parent = self.parent.as_ref().ok_or_else(changed)?;
            unlinkat(
                &parent.file,
                self.path.file_name().ok_or_else(changed)?,
                UnlinkatFlags::RemoveDir,
            )
            .map_err(io)
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

        // A new open description is essential: dup/try_clone shares flock ownership.
        // The kernel releases this nonblocking lock even on process exit.
        pub(crate) fn publication_lock(&self) -> Result<Flock<File>, RunError> {
            self.ensure_attached()?;
            let file: File = openat(&self.file, ".", directory_flags(), Mode::empty())
                .map_err(io)?
                .into();
            if !same(&fstat(&self.file).map_err(io)?, &fstat(&file).map_err(io)?) {
                return Err(changed());
            }
            let lock =
                Flock::lock(file, FlockArg::LockExclusiveNonblock).map_err(|(_, error)| {
                    if error == Errno::EWOULDBLOCK {
                        RunError::Lifecycle("source cache publication is busy; retry")
                    } else {
                        io(error)
                    }
                })?;
            self.ensure_attached()?;
            Ok(lock)
        }

        pub(crate) fn sync_chain(&self) -> Result<(), RunError> {
            self.ensure_attached()?;
            self.file.sync_all()?;
            if let Some(parent) = &self.parent {
                parent.sync_chain()?;
            }
            self.ensure_attached()
        }

        /// Publish complete bytes without ever replacing a destination. A crash
        /// between link and unlink leaves a multiple-link record, which remains
        /// a blocker: no unrelated link is inferred to be disposable.
        pub(crate) fn publish_new(
            self: &Arc<Self>,
            name: &OsStr,
            bytes: &[u8],
            readonly: bool,
        ) -> Result<(), RunError> {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::time::{SystemTime, UNIX_EPOCH};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            component(name)?;
            self.ensure_attached()?;
            if self.open_file(name)?.is_some() {
                return Ok(());
            }
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| changed())?
                .as_nanos();
            let temporary = OsString::from(format!(
                ".source-{}-{nonce}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file: File = openat(
                &self.file,
                temporary.as_os_str(),
                OFlag::O_WRONLY
                    | OFlag::O_CREAT
                    | OFlag::O_EXCL
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_CLOEXEC,
                Mode::S_IRUSR | Mode::S_IWUSR,
            )
            .map_err(io)?
            .into();
            cache_checkpoint("temporary-created")?;
            file.write_all(bytes)?;
            if readonly {
                fchmod(&file, Mode::S_IRUSR).map_err(io)?;
            }
            cache_checkpoint("temporary-written")?;
            file.sync_all()?;
            let pinned = PinnedFile {
                file,
                parent: self.clone(),
                name: temporary.clone(),
            };
            pinned.ensure_attached()?;
            cache_checkpoint("temporary-synced")?;
            pinned.ensure_attached()?;
            linkat(
                &self.file,
                temporary.as_os_str(),
                &self.file,
                name,
                AtFlags::empty(),
            )
            .map_err(io)?;
            cache_checkpoint("temporary-linked")?;
            let expected = fstat(&pinned.file).map_err(io)?;
            for leaf in [temporary.as_os_str(), name] {
                let current = stat(self, leaf)?.ok_or_else(changed)?;
                if !same(&expected, &current) || current.st_nlink != 2 {
                    return Err(changed());
                }
            }
            self.ensure_attached()?;
            unlinkat(
                &self.file,
                temporary.as_os_str(),
                UnlinkatFlags::NoRemoveDir,
            )
            .map_err(io)?;
            cache_checkpoint("temporary-unlinked")?;
            let published = PinnedFile {
                file: pinned.file,
                parent: self.clone(),
                name: name.to_owned(),
            };
            published.sync()?;
            cache_checkpoint("before-directory-sync")?;
            self.sync_chain()?;
            cache_checkpoint("after-directory-sync")?;
            published.ensure_attached()
        }

        #[cfg(feature = "local-bundle-provider")]
        pub(crate) fn sync_bounded_tree(
            self: &Arc<Self>,
            limit: u64,
            deadline: std::time::Instant,
        ) -> Result<(), RunError> {
            let mut remaining = MAX_TREE_ENTRIES;
            let snapshot = Node::scan(self, 0, &mut remaining, Some(deadline))?;
            snapshot.sync_bounded(self, &mut 0, limit, deadline)?;
            self.sync_chain()
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
            let snapshot = Node::scan(self, 0, &mut remaining, None)?;
            let tree = Tree {
                directory: self.clone(),
                snapshot,
            };
            tree.ensure_attached()?;
            Ok(tree)
        }
    }

    impl PinnedFile {
        pub(crate) fn identity_chain(&self) -> Result<Vec<(u64, u64)>, RunError> {
            self.ensure_attached()?;
            use std::os::unix::fs::MetadataExt;
            let metadata = self.file.metadata()?;
            let mut identity = vec![(metadata.dev(), metadata.ino())];
            let mut ancestor = Some(self.parent.as_ref());
            while let Some(directory) = ancestor {
                let metadata = directory.file.metadata()?;
                identity.push((metadata.dev(), metadata.ino()));
                ancestor = directory.parent.as_deref();
            }
            Ok(identity)
        }

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

        pub(crate) fn sync(&self) -> Result<(), RunError> {
            self.ensure_attached()?;
            self.file.sync_all()?;
            self.ensure_attached()
        }

        pub(crate) fn digest(&self) -> Result<String, RunError> {
            use sha2::{Digest, Sha256};
            self.ensure_attached()?;
            let before = fstat(&self.file).map_err(io)?;
            let mut remaining = u64::try_from(before.st_size).map_err(|_| changed())?;
            let mut file = &self.file;
            file.seek(SeekFrom::Start(0))?;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            while remaining > 0 {
                let length =
                    usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| changed())?;
                let read = file.read(&mut buffer[..length])?;
                if read == 0 {
                    return Err(changed());
                }
                hash.update(&buffer[..read]);
                remaining -= read as u64;
                cache_checkpoint("digest-chunk")?;
            }
            self.ensure_unchanged(&before)?;
            let hex: String = hash
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            Ok(format!("sha256:{hex}"))
        }

        fn ensure_unchanged(&self, before: &FileStat) -> Result<(), RunError> {
            self.ensure_attached()?;
            let after = fstat(&self.file).map_err(io)?;
            if !same(before, &after)
                || before.st_nlink != after.st_nlink
                || before.st_size != after.st_size
                || before.st_mtime != after.st_mtime
                || before.st_mtime_nsec != after.st_mtime_nsec
                || before.st_ctime != after.st_ctime
                || before.st_ctime_nsec != after.st_ctime_nsec
            {
                return Err(changed());
            }
            Ok(())
        }

        // Collapse only runs of JSON whitespace outside strings. This preserves
        // token separation and permits legacy formatting without allocating from
        // attacker-supplied file lengths or whitespace. CPU/I/O is not a quota.
        pub(crate) fn bounded_json(&self, limit: usize) -> Result<Vec<u8>, RunError> {
            self.ensure_attached()?;
            let before = fstat(&self.file).map_err(io)?;
            let mut remaining = u64::try_from(before.st_size).map_err(|_| changed())?;
            let mut file = &self.file;
            file.seek(SeekFrom::Start(0))?;
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            let (mut string, mut escaped, mut whitespace) = (false, false, false);
            while remaining > 0 {
                let length =
                    usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| changed())?;
                let read = file.read(&mut buffer[..length])?;
                if read == 0 {
                    return Err(changed());
                }
                remaining -= read as u64;
                for &byte in &buffer[..read] {
                    if !string && matches!(byte, b' ' | b'\n' | b'\r' | b'\t') {
                        if whitespace {
                            continue;
                        }
                        whitespace = true;
                    } else {
                        whitespace = false;
                    }
                    if bytes.len() == limit {
                        return Err(RunError::Lifecycle(
                            "source cache metadata exceeds expected binding encoding bound",
                        ));
                    }
                    bytes.push(if whitespace { b' ' } else { byte });
                    if string {
                        if escaped {
                            escaped = false;
                        } else if byte == b'\\' {
                            escaped = true;
                        } else if byte == b'"' {
                            string = false;
                        }
                    } else if byte == b'"' {
                        string = true;
                    }
                }
            }
            self.ensure_unchanged(&before)?;
            Ok(bytes)
        }

        #[cfg(feature = "local-bundle-provider")]
        pub(crate) fn set_executable(&self) -> Result<(), RunError> {
            self.ensure_attached()?;
            fchmod(&self.file, Mode::S_IRWXU).map_err(io)?;
            self.sync()
        }

        #[cfg(feature = "local-bundle-provider")]
        pub(crate) fn bounded_bytes(
            &self,
            limit: usize,
            deadline: std::time::Instant,
        ) -> Result<Vec<u8>, RunError> {
            self.ensure_attached()?;
            let before = fstat(&self.file).map_err(io)?;
            let length = usize::try_from(before.st_size).map_err(|_| changed())?;
            if length > limit {
                return Err(RunError::Lifecycle("local bundle file exceeds byte limit"));
            }
            let mut bytes = Vec::new();
            let mut file = &self.file;
            file.seek(SeekFrom::Start(0))?;
            let mut chunk = [0; 64 * 1024];
            while bytes.len() < length {
                if std::time::Instant::now() >= deadline {
                    return Err(RunError::Lifecycle(
                        "local bundle operation deadline expired",
                    ));
                }
                let amount = (length - bytes.len()).min(chunk.len());
                let read = file.read(&mut chunk[..amount])?;
                if read == 0 {
                    return Err(changed());
                }
                bytes.extend_from_slice(&chunk[..read]);
            }
            self.ensure_unchanged(&before)?;
            if std::time::Instant::now() >= deadline {
                return Err(RunError::Lifecycle(
                    "local bundle operation deadline expired",
                ));
            }
            Ok(bytes)
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
            deadline: Option<std::time::Instant>,
        ) -> Result<Self, RunError> {
            if deadline.is_some_and(|end| std::time::Instant::now() >= end) {
                return Err(RunError::Lifecycle("filesystem traversal deadline expired"));
            }
            directory.ensure_attached()?;
            let mut entries =
                Dir::openat(&directory.file, ".", directory_flags(), Mode::empty()).map_err(io)?;
            let mut names = Vec::new();
            for entry in entries.iter() {
                if deadline.is_some_and(|end| std::time::Instant::now() >= end) {
                    return Err(RunError::Lifecycle("filesystem traversal deadline expired"));
                }
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
                        let snapshot = Self::scan(&child, depth + 1, remaining, deadline)?;
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

        #[cfg(feature = "local-bundle-provider")]
        fn sync_bounded(
            &self,
            directory: &Arc<Directory>,
            bytes: &mut u64,
            limit: u64,
            deadline: std::time::Instant,
        ) -> Result<(), RunError> {
            for entry in &self.files {
                if std::time::Instant::now() >= deadline {
                    return Err(RunError::Lifecycle(
                        "filesystem synchronization deadline expired",
                    ));
                }
                let file = entry.file(directory)?;
                let length =
                    u64::try_from(fstat(&file.file).map_err(io)?.st_size).map_err(|_| changed())?;
                *bytes = bytes.checked_add(length).ok_or_else(budget_exceeded)?;
                if *bytes > limit {
                    return Err(RunError::Lifecycle(
                        "observed Git disk bytes exceed configured limit",
                    ));
                }
                file.sync()?;
            }
            for (entry, node) in &self.children {
                node.sync_bounded(&entry.directory(directory)?, bytes, limit, deadline)?;
            }
            if std::time::Instant::now() >= deadline {
                return Err(RunError::Lifecycle(
                    "filesystem synchronization deadline expired",
                ));
            }
            directory.sync_chain()
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
        pub(crate) fn overlaps(&self, _: &Self) -> Result<bool, RunError> {
            unsupported()
        }
        pub(crate) fn identity(&self) -> Result<String, RunError> {
            unsupported()
        }
        pub(crate) fn remove_empty(self: &Arc<Self>) -> Result<(), RunError> {
            unsupported()
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
        pub(crate) fn publication_lock(&self) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn sync_chain(&self) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn publish_new(
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
        pub(crate) fn identity_chain(&self) -> Result<Vec<(u64, u64)>, RunError> {
            unsupported()
        }
        pub(crate) fn ensure_attached(&self) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn sync(&self) -> Result<(), RunError> {
            unsupported()
        }
        pub(crate) fn bounded_json(&self, _: usize) -> Result<Vec<u8>, RunError> {
            unsupported()
        }
        pub(crate) fn digest(&self) -> Result<String, RunError> {
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

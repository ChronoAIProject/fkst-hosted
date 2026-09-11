//! Explicit trusted local embedding; never registered by production admission.
//! Provider records are local effect evidence; the manager Journal owns lifecycle.
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::*;

#[path = "local_bundle_process.rs"]
mod process;
use process::{GitRunner, GitStep};

const RECORD_BYTES: usize = 128 * 1024;
const MAX_DEPTH: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalBundleObject {
    pub binding: TrustedLocalSourceBinding,
    pub file_name: String,
    pub object_format: String,
    pub revision_strategy: String,
}

#[derive(Clone)]
pub struct LocalBundleLimits {
    pub max_bundle_bytes: usize,
    pub max_blob_bytes: usize,
    pub max_tree_bytes: usize,
    pub max_files: usize,
    pub max_objects: usize,
    pub max_output_bytes: usize,
    pub max_total_output_bytes: usize,
    /// Observed after import, not a hard bound on Git's transient disk use.
    pub max_git_disk_bytes: u64,
    pub operation_timeout: Duration,
}

impl Default for LocalBundleLimits {
    fn default() -> Self {
        Self {
            max_bundle_bytes: 16 * 1024 * 1024,
            max_blob_bytes: 4 * 1024 * 1024,
            max_tree_bytes: 32 * 1024 * 1024,
            max_files: 1000,
            max_objects: 10_000,
            max_output_bytes: 4 * 1024 * 1024,
            max_total_output_bytes: 64 * 1024 * 1024,
            max_git_disk_bytes: 64 * 1024 * 1024,
            operation_timeout: Duration::from_secs(30),
        }
    }
}

pub struct LocalBundleConfig {
    pub source_store: PathBuf,
    pub state_root: PathBuf,
    pub workspace_root: PathBuf,
    pub cache_root: PathBuf,
    pub journal_parent: PathBuf,
    pub additional_writable_roots: Vec<PathBuf>,
    pub git_executable: PathBuf,
    pub scope: String,
    pub objects: Vec<LocalBundleObject>,
    pub limits: LocalBundleLimits,
}

struct Adapter {
    source: Arc<Directory>,
    state: Arc<Directory>,
    workspace: Arc<Directory>,
    cache: Arc<Directory>,
    protected: Vec<Arc<Directory>>,
    executable: PinnedFile,
    git: PathBuf,
    scope: String,
    identity: String,
    objects: Vec<LocalBundleObject>,
    limits: LocalBundleLimits,
}

pub struct LocalBundleSource(Arc<Adapter>);
pub struct LocalBundleWorkspace(Arc<Adapter>);

/// All roots must already exist and be private to the embedding's effective UID.
/// Construction performs reads only, including every validation failure.
pub fn local_bundle_providers(
    config: LocalBundleConfig,
) -> Result<(LocalBundleSource, LocalBundleWorkspace), RunError> {
    validate_limits(&config.limits)?;
    if config.scope.is_empty()
        || config.scope.len() > 128
        || config.objects.is_empty()
        || config.objects.len() > 128
        || config.additional_writable_roots.len() > 32
    {
        return fail("invalid local bundle registration or scope limits");
    }
    let mut identities = BTreeSet::new();
    for object in &config.objects {
        validate_source_expectations(&object.binding)?;
        component(&object.file_name)?;
        if object.object_format != "git_bundle"
            || object.revision_strategy != "exact_commit"
            || !matches!(
                object.binding.expected_revision,
                ImmutableRevision::GitCommit(_)
            )
        {
            return fail("local provider supports only git_bundle and exact_commit");
        }
        if encode_cache(object)?.len() > 8192
            || !identities.insert(object.binding.source_object_id.clone())
        {
            return fail("duplicate or oversized local bundle registration");
        }
    }
    let source = private_root(&config.source_store)?;
    let state = private_root(&config.state_root)?;
    let workspace = private_root(&config.workspace_root)?;
    let cache = private_root(&config.cache_root)?;
    let mut protected = vec![private_root(&config.journal_parent)?];
    for path in &config.additional_writable_roots {
        protected.push(private_root(path)?);
    }
    let roots: Vec<_> = [&source, &state, &workspace, &cache]
        .into_iter()
        .chain(protected.iter())
        .collect();
    for (index, root) in roots.iter().enumerate() {
        for other in &roots[index + 1..] {
            if root.overlaps(other)? {
                return fail("local bundle roots must be pairwise disjoint");
            }
        }
    }
    let parent = Directory::open_existing(
        config
            .git_executable
            .parent()
            .ok_or(RunError::Lifecycle("Git executable parent missing"))?,
    )?;
    let executable = parent
        .open_file(file_name(&config.git_executable)?)?
        .ok_or(RunError::Lifecycle("Git executable missing"))?;
    let metadata = std::fs::symlink_metadata(&config.git_executable)?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
        || ![0, nix::unistd::geteuid().as_raw()].contains(&metadata.uid())
    {
        return fail("Git executable must be Host-selected and protected from other users");
    }
    executable.ensure_attached()?;
    let root_ids = roots
        .iter()
        .map(|root| root.identity())
        .collect::<Result<Vec<_>, _>>()?;
    let identity = sha256_digest(&encode_cache(&(
        "local-bundle/v1",
        &config.scope,
        root_ids,
        executable.identity_chain()?,
    ))?);
    let adapter = Arc::new(Adapter {
        source,
        state,
        workspace,
        cache,
        protected,
        executable,
        git: config.git_executable,
        scope: config.scope,
        identity,
        objects: config.objects,
        limits: config.limits,
    });
    Ok((
        LocalBundleSource(adapter.clone()),
        LocalBundleWorkspace(adapter),
    ))
}

fn fail<T>(message: &'static str) -> Result<T, RunError> {
    Err(RunError::Lifecycle(message))
}
fn component(value: &str) -> Result<(), RunError> {
    if value.is_empty()
        || value.len() > 200
        || value == "."
        || value == ".."
        || value
            .bytes()
            .any(|b| b == b'/' || b == b'\\' || b == 0 || b.is_ascii_control())
    {
        return fail("local bundle name must be one confined path component");
    }
    Ok(())
}
fn private_root(path: &Path) -> Result<Arc<Directory>, RunError> {
    let directory = Directory::open_existing(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return fail("local bundle roots must be private Host-owned directories");
    }
    directory.ensure_attached()?;
    Ok(directory)
}
fn validate_limits(limits: &LocalBundleLimits) -> Result<(), RunError> {
    for (value, ceiling) in [
        (limits.max_bundle_bytes, 64 * 1024 * 1024),
        (limits.max_blob_bytes, 16 * 1024 * 1024),
        (limits.max_tree_bytes, 128 * 1024 * 1024),
        (limits.max_files, 4000),
        (limits.max_objects, 50_000),
        (limits.max_output_bytes, 16 * 1024 * 1024),
        (limits.max_total_output_bytes, 256 * 1024 * 1024),
    ] {
        if value == 0 || value > ceiling {
            return fail("local bundle configured limit outside supported range");
        }
    }
    if limits.max_git_disk_bytes == 0
        || limits.max_git_disk_bytes > 256 * 1024 * 1024
        || limits.operation_timeout < Duration::from_millis(10)
        || limits.operation_timeout > Duration::from_secs(120)
    {
        return fail("invalid local bundle disk or duration limit");
    }
    Ok(())
}
fn check_time(deadline: Instant) -> Result<(), RunError> {
    if Instant::now() >= deadline {
        return fail("local bundle operation deadline expired");
    }
    Ok(())
}

fn deadline(timeout: Duration, utc: Option<&str>) -> Result<Instant, RunError> {
    let monotonic = Instant::now();
    let wallclock = SystemTime::now();
    let duration = if let Some(utc) = utc {
        validate_scalar("ISO8601", utc)
            .map_err(|_| RunError::Lifecycle("invalid local bundle deadline"))?;
        if !utc.ends_with('Z')
            || utc.len() < 20
            || utc.as_bytes().get(19) == Some(&b'Z') && utc.len() != 20
        {
            return fail("invalid local bundle deadline");
        }
        let whole = &utc[..19];
        let fraction = if utc.as_bytes().get(19) == Some(&b'.') {
            &utc[20..utc.len() - 1]
        } else {
            ""
        };
        if !fraction.is_empty()
            && (fraction.ends_with('0') || !fraction.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return fail("invalid local bundle deadline fraction");
        }
        let number = |range: std::ops::Range<usize>| {
            whole[range]
                .parse::<u64>()
                .map_err(|_| RunError::Lifecycle("invalid local bundle deadline"))
        };
        let year = number(0..4)?;
        if year < 1970 {
            return fail("local bundle deadline expired");
        }
        let leap =
            |y: u64| y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
        let mut days: u64 = (1970..year).map(|y| if leap(y) { 366 } else { 365 }).sum();
        let month = number(5..7)? as usize;
        let months = [
            31,
            if leap(year) { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        days += months[..month - 1].iter().sum::<u64>() + number(8..10)? - 1;
        let whole_end = UNIX_EPOCH
            + Duration::from_secs(
                days * 86400 + number(11..13)? * 3600 + number(14..16)? * 60 + number(17..19)?,
            );
        let fraction_nanos = if fraction.is_empty() {
            0
        } else {
            let precision = fraction.len().min(9);
            let value = fraction[..precision]
                .parse::<u32>()
                .map_err(|_| RunError::Lifecycle("invalid local bundle deadline fraction"))?;
            value * 10u32.pow(9 - precision as u32)
        };
        let end = whole_end
            .checked_add(Duration::from_nanos(u64::from(fraction_nanos)))
            .ok_or(RunError::Lifecycle("local bundle deadline overflow"))?;
        timeout.min(
            end.duration_since(wallclock)
                .map_err(|_| RunError::Lifecycle("local bundle deadline expired"))?,
        )
    } else {
        timeout
    };
    monotonic
        .checked_add(duration)
        .ok_or(RunError::Lifecycle("local bundle deadline overflow"))
}

impl Adapter {
    fn check(&self, end: Instant) -> Result<(), RunError> {
        check_time(end)?;
        for root in [&self.source, &self.state, &self.workspace, &self.cache]
            .into_iter()
            .chain(self.protected.iter())
        {
            root.ensure_attached()?;
        }
        self.executable.ensure_attached()?;
        check_time(end)
    }
    fn object(&self, binding: &TrustedLocalSourceBinding) -> Result<&LocalBundleObject, RunError> {
        self.objects
            .iter()
            .find(|object| object.binding == *binding)
            .ok_or(RunError::Lifecycle(
                "source is not an exact Host-selected bundle registration",
            ))
    }
    fn intent_object(&self, intent: &WorkspaceIntent) -> Result<&LocalBundleObject, RunError> {
        let binding = intent
            .source_binding
            .as_ref()
            .ok_or(RunError::Lifecycle("local bundle intent binding missing"))?;
        let object = self.object(binding)?;
        validate_scalar("UUID", &intent.run_id)
            .map_err(|_| RunError::Lifecycle("invalid local bundle Run id"))?;
        if intent.generation <= 0
            || intent.stable_key != stable_workspace_key(&intent.run_id, intent.generation)
            || intent.relative_location
                != format!("{}/generation-{}", intent.run_id, intent.generation)
            || intent.provider_scope != self.scope
            || intent.source_object_id != binding.source_object_id
            || intent.source_digest != binding.expected_raw_digest
            || intent.immutable_revision != binding.expected_revision
            || intent.source_provider_identity != binding.expected_provider_identity
            || encode_cache(intent)?.len() > RECORD_BYTES / 4
        {
            return fail(
                "local bundle intent does not match registered source and workspace scope",
            );
        }
        Ok(object)
    }
    fn root(&self, intent: &WorkspaceIntent) -> Result<Arc<Directory>, RunError> {
        self.intent_object(intent)?;
        self.workspace
            .child(intent.run_id.as_ref(), false)?
            .ok_or(RunError::Lifecycle("workspace Run directory missing"))?
            .child(format!("generation-{}", intent.generation).as_ref(), false)?
            .ok_or(RunError::Lifecycle("workspace directory missing"))
    }
    fn name(&self, intent: &WorkspaceIntent, suffix: &str) -> String {
        format!(
            "{}.{suffix}",
            &sha256_digest(intent.stable_key.as_bytes())[7..]
        )
    }
    fn read<T: for<'de> Deserialize<'de>>(
        &self,
        intent: &WorkspaceIntent,
        suffix: &str,
        end: Instant,
    ) -> Result<Option<T>, RunError> {
        self.check(end)?;
        self.state
            .open_file(self.name(intent, suffix).as_ref())?
            .map(|file| {
                let bytes = file.bounded_bytes(RECORD_BYTES, end)?;
                let record = serde_json::from_slice(&bytes)
                    .map_err(|_| RunError::Lifecycle("invalid local bundle provider evidence"))?;
                file.sync()?;
                self.state.sync_chain()?;
                self.check(end)?;
                Ok(record)
            })
            .transpose()
    }
    fn publish(
        &self,
        intent: &WorkspaceIntent,
        suffix: &str,
        value: &impl Serialize,
        end: Instant,
    ) -> Result<(), RunError> {
        self.check(end)?;
        let bytes = encode_cache(value)?;
        if bytes.len() > RECORD_BYTES {
            return fail("local bundle evidence exceeds record limit");
        }
        self.state
            .publish_new(self.name(intent, suffix).as_ref(), &bytes, false)?;
        self.check(end)
    }
    fn completion(
        &self,
        intent: &WorkspaceIntent,
        end: Instant,
    ) -> Result<WorkspaceDiscovery, RunError> {
        let object = self.intent_object(intent)?;
        let Some(attempt) = self.read::<Attempt>(intent, "attempt.json", end)? else {
            if self
                .read::<Completion>(intent, "complete.json", end)?
                .is_some()
                || self
                    .read::<Completion>(intent, "stopped.json", end)?
                    .is_some()
            {
                return Ok(WorkspaceDiscovery::Conflict);
            }
            return Ok(WorkspaceDiscovery::Absent);
        };
        if attempt.adapter != self.identity
            || attempt.intent != *intent
            || attempt.registration != *object
        {
            return Ok(WorkspaceDiscovery::Conflict);
        }
        let Some(complete) = self.read::<Completion>(intent, "complete.json", end)? else {
            return Ok(WorkspaceDiscovery::Unknown);
        };
        if complete.attempt != attempt || complete.resource != attempt.resource() {
            return Ok(WorkspaceDiscovery::Conflict);
        }
        let stopped = self.read::<Completion>(intent, "stopped.json", end)?;
        if let Some(stopped) = stopped {
            if stopped != complete {
                return Ok(WorkspaceDiscovery::Conflict);
            }
            return Ok(WorkspaceDiscovery::Found(Box::new(
                WorkspaceProviderStatusReceipt {
                    resource: complete.resource,
                    status: WorkspaceProviderStatus::Stopped,
                },
            )));
        }
        let root = self.root(intent)?;
        let git = root
            .child(".git".as_ref(), false)?
            .ok_or(RunError::Lifecycle("materialized Git directory missing"))?;
        if root.identity()? != attempt.directory || git.identity()? != complete.git_directory {
            return Ok(WorkspaceDiscovery::Conflict);
        }
        if validate_git_directory(&git, &self.limits, end)? != complete.git_config_digest {
            return Ok(WorkspaceDiscovery::Conflict);
        }
        self.check(end)?;
        Ok(WorkspaceDiscovery::Found(Box::new(
            WorkspaceProviderStatusReceipt {
                resource: complete.resource,
                status: WorkspaceProviderStatus::Active,
            },
        )))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    adapter: String,
    intent: WorkspaceIntent,
    registration: LocalBundleObject,
    directory: String,
}
impl Attempt {
    fn resource(&self) -> WorkspaceResource {
        WorkspaceResource {
            intent: self.intent.clone(),
            provider_identity: sha256_digest(&encode_cache(self).expect("serializable attempt")),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    attempt: Attempt,
    resource: WorkspaceResource,
    git_directory: String,
    git_config_digest: String,
}

impl SourceProvider for LocalBundleSource {
    fn acquire(&mut self, lease: &SourceObjectLease) -> Result<AcquiredSource, RunError> {
        let adapter = &self.0;
        let end = deadline(adapter.limits.operation_timeout, Some(&lease.deadline_utc))?;
        adapter.check(end)?;
        let object = adapter.object(&lease.binding)?;
        let file = adapter
            .source
            .open_file(object.file_name.as_ref())?
            .ok_or(RunError::Lifecycle("registered local bundle missing"))?;
        let bytes = file.bounded_bytes(adapter.limits.max_bundle_bytes, end)?;
        if sha256_digest(&bytes) != object.binding.expected_raw_digest {
            return fail("local bundle raw digest mismatch");
        }
        adapter.check(end)?;
        Ok(AcquiredSource {
            source_object_id: object.binding.source_object_id.clone(),
            immutable_revision: object.binding.expected_revision.clone(),
            provider_scope: object.binding.expected_provider_scope.clone(),
            provider_identity: object.binding.expected_provider_identity.clone(),
            bytes,
        })
    }
}
impl WorkspaceProvider for LocalBundleWorkspace {
    fn scope(&self) -> &str {
        &self.0.scope
    }
    fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
        let end = deadline(self.0.limits.operation_timeout, None)?;
        let _lock = match self.0.state.publication_lock() {
            Ok(lock) => lock,
            Err(RunError::Lifecycle("source cache publication is busy; retry")) => {
                return Ok(WorkspaceDiscovery::RetryableBusy)
            }
            Err(error) => return Err(error),
        };
        self.0.completion(intent, end)
    }
    fn materialize(
        &mut self,
        intent: &WorkspaceIntent,
        source: &Path,
        root_path: &Path,
        revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, RunError> {
        let adapter = &self.0;
        let end = deadline(adapter.limits.operation_timeout, Some(&intent.deadline_utc))?;
        let _lock = adapter.state.publication_lock()?;
        adapter.check(end)?;
        let object = adapter.intent_object(intent)?;
        if *revision != object.binding.expected_revision {
            return fail("local bundle requested revision mismatch");
        }
        let expected_source = adapter.cache.path().join(format!(
            "{}.source",
            &object.binding.expected_raw_digest[7..]
        ));
        if source != expected_source {
            return fail("local bundle source path must name the registered cache object");
        }
        let file = adapter
            .cache
            .open_file(file_name(source)?)?
            .ok_or(RunError::Lifecycle("verified bundle cache missing"))?;
        let bytes = file.bounded_bytes(adapter.limits.max_bundle_bytes, end)?;
        if sha256_digest(&bytes) != object.binding.expected_raw_digest {
            return fail("local bundle raw digest mismatch");
        }
        let root = adapter.root(intent)?;
        if root.path() != root_path {
            return fail("local bundle workspace path is not manager selected");
        }
        match adapter.completion(intent, end)? {
            WorkspaceDiscovery::Absent => (),
            _ => return fail("local bundle effect already attempted; discovery required"),
        }
        let attempt = Attempt {
            adapter: adapter.identity.clone(),
            intent: intent.clone(),
            registration: object.clone(),
            directory: root.identity()?,
        };
        adapter.publish(intent, "attempt.json", &attempt, end)?;
        // The intent remains even if bundle parsing, Git, or publication fails.
        validate_bundle(&bytes, &adapter.limits)?;
        let private = adapter
            .state
            .create_child(adapter.name(intent, "files").as_ref())?;
        private.create_child("template".as_ref())?;
        private.write_new("input.bundle".as_ref(), &bytes, true)?;
        private.sync_chain()?;
        drop(bytes);
        if std::fs::read_dir(root.path())?.next().is_some() {
            return fail("local bundle workspace must initially be empty");
        }
        let mut runner = GitRunner::new(
            &adapter.git,
            root.path(),
            private.path(),
            end,
            &adapter.limits,
        );
        let mut config_digest = None;
        let mut guarded = |runner: &mut GitRunner, step| -> Result<Vec<u8>, RunError> {
            adapter.check(end)?;
            root.ensure_attached()?;
            private.ensure_attached()?;
            if let Some(expected) = &config_digest {
                let git = root
                    .child(".git".as_ref(), false)?
                    .ok_or(RunError::Lifecycle("Git directory disappeared"))?;
                if validate_git_directory(&git, &adapter.limits, end)? != *expected {
                    return fail("Git config changed across verification steps");
                }
            }
            let output = runner.run(step)?;
            adapter.check(end)?;
            root.ensure_attached()?;
            private.ensure_attached()?;
            let git = root
                .child(".git".as_ref(), false)?
                .ok_or(RunError::Lifecycle("Git directory missing after command"))?;
            let actual = validate_git_directory(&git, &adapter.limits, end)?;
            if config_digest
                .as_ref()
                .is_some_and(|expected| *expected != actual)
            {
                return fail("Git config changed during command");
            }
            config_digest = Some(actual);
            Ok(output)
        };
        guarded(&mut runner, GitStep::Init)?;
        let git = root
            .child(".git".as_ref(), false)?
            .ok_or(RunError::Lifecycle(
                "Git did not initialize independent directory",
            ))?;
        guarded(&mut runner, GitStep::Verify)?;
        guarded(&mut runner, GitStep::Import)?;
        git.sync_bounded_tree(adapter.limits.max_git_disk_bytes, end)?;
        guarded(&mut runner, GitStep::Fsck)?;
        let inventory = inventory(
            &guarded(&mut runner, GitStep::ObjectInventory)?,
            &adapter.limits,
        )?;
        let ImmutableRevision::GitCommit(commit) = revision else {
            return fail("only exact Git commit supported");
        };
        if guarded(&mut runner, GitStep::CommitType(commit))? != b"commit\n" {
            return fail("requested Git object is not a commit");
        }
        let files = tree(
            &guarded(&mut runner, GitStep::Tree(commit))?,
            &inventory,
            &adapter.limits,
        )?;
        // Validation of all paths/modes precedes writing any source content.
        for entry in &files {
            check_time(end)?;
            let blob = guarded(&mut runner, GitStep::Blob(&entry.object))?;
            if blob.len() != entry.size {
                return fail("Git blob size differs from verified inventory");
            }
            let mut directory = root.clone();
            let parts: Vec<_> = entry.path.split('/').collect();
            for part in &parts[..parts.len() - 1] {
                directory = directory
                    .child((*part).as_ref(), true)?
                    .ok_or(RunError::Lifecycle("source directory creation failed"))?;
            }
            directory.write_new(parts[parts.len() - 1].as_ref(), &blob, false)?;
            if entry.executable {
                directory
                    .open_file(parts[parts.len() - 1].as_ref())?
                    .ok_or(RunError::Lifecycle("materialized file missing"))?
                    .set_executable()?;
            }
            directory.sync_chain()?;
        }
        guarded(&mut runner, GitStep::Index(commit))?;
        // init created only a symbolic unborn HEAD. No checkout operation runs.
        guarded(&mut runner, GitStep::Detach(commit))?;
        git.sync_chain()?;
        root.tree_with_reserved_entries(1)?.ensure_attached()?;
        git.sync_bounded_tree(adapter.limits.max_git_disk_bytes, end)?;
        let complete = Completion {
            resource: attempt.resource(),
            attempt,
            git_directory: git.identity()?,
            git_config_digest: validate_git_directory(&git, &adapter.limits, end)?,
        };
        adapter.publish(intent, "complete.json", &complete, end)?;
        match adapter.completion(intent, end)? {
            WorkspaceDiscovery::Found(receipt)
                if receipt.status == WorkspaceProviderStatus::Active =>
            {
                Ok(WorkspaceMaterialization {
                    resource: receipt.resource,
                })
            }
            _ => fail("local bundle completion could not be established"),
        }
    }
    fn status(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
        match self.discover(&resource.intent)? {
            WorkspaceDiscovery::Found(receipt) if receipt.resource == *resource => Ok(*receipt),
            _ => Ok(WorkspaceProviderStatusReceipt {
                resource: resource.clone(),
                status: WorkspaceProviderStatus::Unknown,
            }),
        }
    }
    fn stop(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStopReceipt, RunError> {
        let adapter = &self.0;
        let end = deadline(adapter.limits.operation_timeout, None)?;
        let _lock = adapter.state.publication_lock()?;
        match adapter.completion(&resource.intent, end)? {
            WorkspaceDiscovery::Found(receipt) if receipt.resource == *resource => {
                let complete = adapter
                    .read::<Completion>(&resource.intent, "complete.json", end)?
                    .ok_or(RunError::Lifecycle("local bundle completion missing"))?;
                adapter.publish(&resource.intent, "stopped.json", &complete, end)?;
                if adapter.read::<Completion>(&resource.intent, "stopped.json", end)?
                    != Some(complete)
                {
                    return fail("local bundle stop evidence mismatch");
                }
                Ok(WorkspaceProviderStopReceipt {
                    resource: resource.clone(),
                    stopped: true,
                })
            }
            _ => fail("local bundle stop ownership unresolved"),
        }
    }
}

fn validate_bundle(bytes: &[u8], limits: &LocalBundleLimits) -> Result<(), RunError> {
    let split = bytes
        .windows(2)
        .take(64 * 1024)
        .position(|pair| pair == b"\n\n")
        .ok_or(RunError::Lifecycle(
            "local bundle header missing or too large",
        ))?;
    let header = std::str::from_utf8(&bytes[..split])
        .map_err(|_| RunError::Lifecycle("invalid bundle header"))?;
    let mut lines = header.lines();
    if !matches!(lines.next(), Some("# v2 git bundle" | "# v3 git bundle")) {
        return fail("unsupported Git bundle version");
    }
    let mut refs = 0;
    for line in lines {
        if line.starts_with('-') {
            return fail("Git bundle prerequisites are unsupported; standalone closure required");
        }
        if line == "@object-format=sha1" {
            continue;
        }
        let Some((id, name)) = line.split_once(' ') else {
            return fail("unsupported bundle capability or reference");
        };
        ImmutableRevision::GitCommit(id.into()).validate()?;
        if name.is_empty() || name.len() > 1024 {
            return fail("invalid bundle reference");
        }
        refs += 1;
        if refs > limits.max_objects {
            return fail("bundle reference count exceeded");
        }
    }
    let pack = bytes
        .get(split + 2..split + 14)
        .ok_or(RunError::Lifecycle("bundle pack header missing"))?;
    let version = u32::from_be_bytes(pack[4..8].try_into().expect("four bytes"));
    let count = u32::from_be_bytes(pack[8..12].try_into().expect("four bytes")) as usize;
    if refs == 0
        || &pack[..4] != b"PACK"
        || ![2, 3].contains(&version)
        || count == 0
        || count > limits.max_objects
    {
        return fail("bundle pack version or object count outside supported limits");
    }
    Ok(())
}

fn inventory(
    bytes: &[u8],
    limits: &LocalBundleLimits,
) -> Result<BTreeMap<String, (String, usize)>, RunError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| RunError::Lifecycle("invalid Git object inventory"))?;
    let mut objects = BTreeMap::new();
    let mut total: usize = 0;
    for line in text.lines() {
        let fields: Vec<_> = line.split(' ').collect();
        if fields.len() != 3 {
            return fail("invalid Git object inventory entry");
        }
        ImmutableRevision::GitCommit(fields[0].into()).validate()?;
        if !["commit", "tree", "blob", "tag"].contains(&fields[1]) {
            return fail("unsupported Git object type");
        }
        let size: usize = fields[2]
            .parse()
            .map_err(|_| RunError::Lifecycle("invalid Git object size"))?;
        total = total
            .checked_add(size)
            .ok_or(RunError::Lifecycle("Git expanded object sizes overflow"))?;
        if size > limits.max_blob_bytes
            || total > limits.max_tree_bytes
            || objects.len() >= limits.max_objects
        {
            return fail("Git expanded objects exceed supported limits");
        }
        if objects
            .insert(fields[0].into(), (fields[1].into(), size))
            .is_some()
        {
            return fail("duplicate Git object inventory");
        }
    }
    Ok(objects)
}
struct SourceFile {
    object: String,
    path: String,
    size: usize,
    executable: bool,
}
fn tree(
    bytes: &[u8],
    objects: &BTreeMap<String, (String, usize)>,
    limits: &LocalBundleLimits,
) -> Result<Vec<SourceFile>, RunError> {
    let mut files = Vec::new();
    let mut paths = BTreeSet::new();
    let mut spellings = BTreeMap::new();
    let mut expanded: usize = 0;
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return fail("truncated Git tree listing");
    }
    for entry in bytes.split(|&b| b == 0).filter(|entry| !entry.is_empty()) {
        let entry = std::str::from_utf8(entry)
            .map_err(|_| RunError::Lifecycle("non-UTF8 source paths unsupported"))?;
        let (metadata, path) = entry
            .split_once('\t')
            .ok_or(RunError::Lifecycle("invalid Git tree entry"))?;
        let fields: Vec<_> = metadata.split(' ').collect();
        if fields.len() != 3 || fields[1] != "blob" || !["100644", "100755"].contains(&fields[0]) {
            return fail("source tree contains unsupported mode, symlink, or gitlink");
        }
        let mut depth = 0;
        let mut prefix = String::new();
        for part in path.split('/') {
            component(part)?;
            depth += 1;
            if !part.is_ascii() || part.ends_with(['.', ' ']) || part.contains(':') {
                return fail("source paths require portable ASCII components without trailing dots or spaces");
            }
            if part.eq_ignore_ascii_case(".git") || part.eq_ignore_ascii_case(WORKSPACE_MARKER) {
                return fail("source path uses reserved workspace metadata name");
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if spellings
                .insert(prefix.to_lowercase(), prefix.clone())
                .is_some_and(|old| old != prefix)
            {
                return fail("case-folding source directory collision");
            }
        }
        if depth > MAX_DEPTH
            || path.len() > 2048
            || files.len() >= limits.max_files
            || !paths.insert(path.to_lowercase())
        {
            return fail("source tree depth, file count, or path collision unsupported");
        }
        let (kind, size) = objects
            .get(fields[2])
            .ok_or(RunError::Lifecycle("source tree references missing object"))?;
        if kind != "blob" {
            return fail("source tree object is not a blob");
        }
        expanded = expanded
            .checked_add(*size)
            .ok_or(RunError::Lifecycle("source expanded bytes overflow"))?;
        if expanded > limits.max_tree_bytes {
            return fail("source materialized bytes exceed limit");
        }
        files.push(SourceFile {
            object: fields[2].into(),
            path: path.into(),
            size: *size,
            executable: fields[0] == "100755",
        });
    }
    Ok(files)
}

fn validate_git_directory(
    git: &Arc<Directory>,
    limits: &LocalBundleLimits,
    end: Instant,
) -> Result<String, RunError> {
    git.sync_bounded_tree(limits.max_git_disk_bytes, end)?;
    for name in ["config.worktree", "shallow"] {
        if git.open_file(name.as_ref())?.is_some() {
            return fail("unsupported Git extension metadata");
        }
    }
    if let Some(info) = git.child("info".as_ref(), false)? {
        if info.open_file("grafts".as_ref())?.is_some() {
            return fail("Git grafts are forbidden");
        }
    }
    if let Some(refs) = git.child("refs".as_ref(), false)? {
        if refs.child("replace".as_ref(), false)?.is_some() {
            return fail("Git replace refs are forbidden");
        }
    }
    let objects = git
        .child("objects".as_ref(), false)?
        .ok_or(RunError::Lifecycle("Git objects directory missing"))?;
    if let Some(info) = objects.child("info".as_ref(), false)? {
        for name in ["alternates", "http-alternates"] {
            if info.open_file(name.as_ref())?.is_some() {
                return fail("Git alternates are forbidden");
            }
        }
    }
    let config = git
        .open_file("config".as_ref())?
        .ok_or(RunError::Lifecycle("Git config missing"))?;
    Ok(sha256_digest(&config.bounded_bytes(64 * 1024, end)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn raw_tree_parser_rejects_traversal_modes_reserved_names_and_collisions() {
        let objects = BTreeMap::from([(ID.into(), ("blob".into(), 1))]);
        let limits = LocalBundleLimits::default();
        for path in [
            "../escape",
            "/absolute",
            "a/../b",
            "a//b",
            ".git/config",
            ".GIT/config",
            ".git./config",
            ".fkst-workspace.json",
            "bad\\name",
            "bad\nname",
            "café",
            "drive:name",
        ] {
            assert!(
                tree(
                    format!("100644 blob {ID}\t{path}\0").as_bytes(),
                    &objects,
                    &limits
                )
                .is_err(),
                "{path}"
            );
        }
        for mode in ["120000", "160000", "100664", "040000"] {
            assert!(
                tree(
                    format!("{mode} blob {ID}\tfile\0").as_bytes(),
                    &objects,
                    &limits
                )
                .is_err(),
                "{mode}"
            );
        }
        for paths in [("a", "A"), ("Foo/a", "foo/b")] {
            assert!(tree(
                format!(
                    "100644 blob {ID}\t{}\0 100644 blob {ID}\t{}\0",
                    paths.0, paths.1
                )
                .replace("\0 ", "\0")
                .as_bytes(),
                &objects,
                &limits
            )
            .is_err());
        }
    }

    #[test]
    fn tree_and_inventory_limits_accept_boundary_and_reject_one_more() {
        let mut limits = LocalBundleLimits {
            max_blob_bytes: 2,
            max_tree_bytes: 2,
            max_files: 1,
            max_objects: 1,
            ..LocalBundleLimits::default()
        };
        let objects = inventory(format!("{ID} blob 2\n").as_bytes(), &limits).unwrap();
        let listing = format!("100644 blob {ID}\tfile\0");
        assert_eq!(
            tree(listing.as_bytes(), &objects, &limits).unwrap().len(),
            1
        );
        assert!(inventory(format!("{ID} blob 3\n").as_bytes(), &limits).is_err());
        assert!(tree(
            format!("{listing}100644 blob {ID}\tother\0").as_bytes(),
            &objects,
            &limits
        )
        .is_err());
        for depth in [MAX_DEPTH, MAX_DEPTH + 1] {
            let path = std::iter::repeat_n("a", depth)
                .collect::<Vec<_>>()
                .join("/");
            assert_eq!(
                tree(
                    format!("100644 blob {ID}\t{path}\0").as_bytes(),
                    &objects,
                    &limits
                )
                .is_ok(),
                depth == MAX_DEPTH
            );
        }
        limits.max_blob_bytes = usize::MAX;
        assert!(validate_limits(&limits).is_err());
        assert!(deadline(Duration::from_secs(1), Some("2000-01-01T00:00:00Z")).is_err());
        assert!(deadline(Duration::from_secs(1), Some("2099-01-01T00:00:00.001Z")).is_ok());
    }
}

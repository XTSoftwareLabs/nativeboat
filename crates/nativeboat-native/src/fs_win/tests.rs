use super::*;
use core::mem::{align_of, offset_of, size_of};
use std::os::windows::fs::MetadataExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct Directory(std::path::PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nativeboat-fs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.0
            .join(name)
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn tty_queries_share_the_public_filesystem_descriptor_table() {
    use crate::{Key, Realm, Value, ops};

    let directory = Directory::new();
    std::fs::write(directory.0.join("tty.txt"), b"regular file").unwrap();
    let mut realm = Realm::new();
    let tty = crate::modules::load(&mut realm, "tty").unwrap();
    let isatty = ops::get(&mut realm, &tty, &Key::name("isatty")).unwrap();
    // Querying before fs is loaded must not create a separate descriptor table.
    assert!(matches!(
        ops::call(&mut realm, &isatty, Value::Undefined, &[Value::Number(3.0)]).unwrap(),
        Value::Boolean(false)
    ));
    let filesystem = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let open = ops::get(&mut realm, &filesystem, &Key::name("open")).unwrap();
    let fd = ops::call(
        &mut realm,
        &open,
        Value::Undefined,
        &[
            Value::str(directory.0.join("tty.txt").to_str().unwrap()),
            Value::Number(0.0),
            Value::Number(0.0),
        ],
    )
    .unwrap();
    assert!(matches!(
        ops::call(
            &mut realm,
            &isatty,
            Value::Undefined,
            std::slice::from_ref(&fd),
        )
        .unwrap(),
        Value::Boolean(false)
    ));
    {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetConsoleMode,
            unsafe extern "system" fn(Handle, *mut u32) -> i32
        );
        query.expect().once().returning(|handle, _| {
            assert!(!handle.is_null());
            1
        });
        assert!(matches!(
            ops::call(
                &mut realm,
                &isatty,
                Value::Undefined,
                std::slice::from_ref(&fd),
            )
            .unwrap(),
            Value::Boolean(true)
        ));
    }
    let close = ops::get(&mut realm, &filesystem, &Key::name("close")).unwrap();
    ops::call(
        &mut realm,
        &close,
        Value::Undefined,
        std::slice::from_ref(&fd),
    )
    .unwrap();
    assert!(matches!(
        ops::call(&mut realm, &isatty, Value::Undefined, &[fd]).unwrap(),
        Value::Boolean(false)
    ));
}

#[test]
fn directory_watch_reports_unicode_names_and_cancels_pending_reads() {
    use std::{sync::Arc, thread, time::Duration};

    let dir = Directory::new();
    let handle = watch_open(&dir.path("")).unwrap().handle;
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let handle_value = handle as usize;
    let pending = thread::spawn(move || {
        // SAFETY: the test keeps this handle open until the worker joins.
        unsafe { watch_read(handle_value as Handle, false, &worker_cancelled) }
    });
    thread::sleep(Duration::from_millis(80));
    std::fs::write(dir.0.join("新しい.txt"), b"change").unwrap();
    let changes = pending.join().unwrap().unwrap().unwrap();
    assert!(changes.iter().any(|(action, name)| {
        *action == 1 && String::from_utf16_lossy(name) == "新しい.txt"
    }));
    // SAFETY: this test owns the open handle and no read is pending.
    unsafe { watch_cancel(handle) };
    watch_close(handle);

    let second = watch_open(&dir.path("")).unwrap().handle;
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let handle_value = second as usize;
    let pending = thread::spawn(move || {
        // SAFETY: cancellation is awaited by joining before the handle closes.
        unsafe { watch_read(handle_value as Handle, false, &worker_cancelled) }
    });
    thread::sleep(Duration::from_millis(50));
    cancelled.store(true, Ordering::Release);
    // SAFETY: this test owns the open handle and is canceling its pending read.
    unsafe { watch_cancel(second) };
    assert!(pending.join().unwrap().unwrap().is_none());
    watch_close(second);
    assert!(watch_open(&dir.path("missing")).is_err());
}

#[test]
fn watcher_setup_and_completion_failures_release_their_owned_events() {
    let directory = Directory::new();
    {
        let mut session = shimforge::Session::new();
        let open = shimforge::mock!(
            session,
            CreateFileW,
            unsafe extern "system" fn(
                *const u16,
                u32,
                u32,
                *mut c_void,
                u32,
                u32,
                Handle,
            ) -> Handle
        );
        open.expect()
            .once()
            .returning(|_, _, _, _, _, _, _| INVALID_HANDLE);
        let error = shimforge::mock!(session, last_error, fn() -> u32);
        error.expect().once().returning(|| 5);
        assert!(matches!(watch_open(&directory.path("")), Err(UV_EPERM)));
    }
    let handle = watch_open(&directory.path("")).unwrap().handle;
    {
        let mut session = shimforge::Session::new();
        let event = shimforge::mock!(
            session,
            CreateEventW,
            unsafe extern "system" fn(*mut c_void, i32, i32, *const u16) -> Handle
        );
        event
            .expect()
            .once()
            .returning(|_, _, _, _| core::ptr::null_mut());
        let error = shimforge::mock!(session, last_error, fn() -> u32);
        error.expect().once().returning(|| 5);
        assert_eq!(
            // SAFETY: this test owns the open directory until every call returns.
            unsafe { watch_read(handle, false, &Arc::new(AtomicBool::new(false))) },
            Err(UV_EPERM)
        );
    }
    for scenario in 0..5 {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut session = shimforge::Session::new();
        // No real overlapped I/O is issued in these failure simulations.
        let read = shimforge::mock!(
            session,
            ReadDirectoryChangesW,
            unsafe extern "system" fn(
                Handle,
                *mut u8,
                u32,
                i32,
                u32,
                *mut u32,
                *mut Overlapped,
                Option<unsafe extern "system" fn(u32, u32, *mut Overlapped)>,
            ) -> i32
        );
        read.expect().once().returning(|_, _, _, _, _, _, _, _| 1);
        let waits = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count = waits.clone();
        let cancelling = cancelled.clone();
        let wait = shimforge::mock!(
            session,
            WaitForSingleObject,
            unsafe extern "system" fn(Handle, u32) -> u32
        );
        wait.expect()
            .times(if scenario <= 2 { 2 } else { 1 })
            .returning(move |_, timeout| {
                if count.fetch_add(1, Ordering::Relaxed) != 0 {
                    return WAIT_OBJECT_0;
                }
                assert_eq!(timeout, 100);
                match scenario {
                    0 => {
                        cancelling.store(true, Ordering::Release);
                        WAIT_TIMEOUT
                    }
                    1 => WAIT_FAILED,
                    2 => 0x80,
                    _ => WAIT_OBJECT_0,
                }
            });
        let cancel = shimforge::mock!(
            session,
            CancelIoEx,
            unsafe extern "system" fn(Handle, *mut Overlapped) -> i32
        );
        if scenario <= 2 {
            cancel.expect().once().returning(|_, _| 1);
        }
        let completion = shimforge::mock!(
            session,
            GetOverlappedResult,
            unsafe extern "system" fn(Handle, *mut Overlapped, *mut u32, i32) -> i32
        );
        if scenario >= 3 {
            let cancelling = cancelled.clone();
            completion.expect().once().returning(move |_, _, _, _| {
                if scenario == 4 {
                    cancelling.store(true, Ordering::Release);
                }
                0
            });
        }
        let error = shimforge::mock!(session, last_error, fn() -> u32);
        if scenario == 1 || scenario >= 3 {
            error.expect().once().returning(|| 5);
        }
        // SAFETY: the directory is retained, and no mock leaves pending I/O.
        let result = unsafe { watch_read(handle, false, &cancelled) };
        assert_eq!(
            result,
            match scenario {
                0 | 4 => Ok(None),
                2 => Err(UV_EINVAL),
                _ => Err(UV_EPERM),
            }
        );
    }
    watch_close(handle);
}

#[test]
fn directory_watch_notification_records_validate_offsets_and_names() {
    let mut records = vec![0_u8; 30];
    records[0..4].copy_from_slice(&16_u32.to_le_bytes());
    records[4..8].copy_from_slice(&1_u32.to_le_bytes());
    records[8..12].copy_from_slice(&2_u32.to_le_bytes());
    records[12..14].copy_from_slice(&(u16::from(b'a')).to_le_bytes());
    records[16..20].copy_from_slice(&0_u32.to_le_bytes());
    records[20..24].copy_from_slice(&3_u32.to_le_bytes());
    records[24..28].copy_from_slice(&2_u32.to_le_bytes());
    records[28..30].copy_from_slice(&(u16::from(b'b')).to_le_bytes());
    assert_eq!(
        parse_watch_changes(&records, records.len()).unwrap(),
        vec![(1, vec![u16::from(b'a')]), (3, vec![u16::from(b'b')])]
    );
    for (buffer, used) in [
        (records.as_slice(), records.len() + 1),
        (records.as_slice(), 11),
    ] {
        assert_eq!(parse_watch_changes(buffer, used), Err(UV_EINVAL));
    }
    for name_bytes in [1_u32, 6_u32] {
        let mut malformed = vec![0_u8; 16];
        malformed[8..12].copy_from_slice(&name_bytes.to_le_bytes());
        assert_eq!(
            parse_watch_changes(&malformed, malformed.len()),
            Err(UV_EINVAL)
        );
    }
    for next in [4_u32, 17_u32] {
        let mut malformed = vec![0_u8; 12];
        malformed[0..4].copy_from_slice(&next.to_le_bytes());
        assert_eq!(
            parse_watch_changes(&malformed, malformed.len()),
            Err(UV_EINVAL)
        );
    }
}

#[test]
fn watched_file_names_use_windows_unicode_case_folding() {
    assert!(watch_name_matches(
        &"WatchedÄ原稿".encode_utf16().collect::<Vec<_>>(),
        &"watchedä原稿".encode_utf16().collect::<Vec<_>>()
    ));
    assert!(!watch_name_matches(
        &"watched-a".encode_utf16().collect::<Vec<_>>(),
        &"watched-b".encode_utf16().collect::<Vec<_>>()
    ));
}

#[test]
fn file_watch_paths_preserve_parents_and_support_current_directory_names() {
    let path = |value: &str| value.encode_utf16().chain([0]).collect::<Vec<_>>();
    assert_eq!(
        watch_parent_and_name(&path("note.txt")),
        (path("."), "note.txt".encode_utf16().collect())
    );
    assert_eq!(
        watch_parent_and_name(&path(r"C:\folder\note.txt")),
        (path(r"C:\folder\"), "note.txt".encode_utf16().collect())
    );
    assert_eq!(
        watch_parent_and_name(&path(r"C:\note.txt")),
        (path(r"C:\"), "note.txt".encode_utf16().collect())
    );
}

#[test]
fn file_information_matches_the_windows_abi_and_live_metadata() {
    assert_eq!((size_of::<FileTime>(), align_of::<FileTime>()), (8, 4));
    assert_eq!(
        (
            size_of::<HandleInformation>(),
            align_of::<HandleInformation>()
        ),
        (52, 4)
    );
    assert_eq!(offset_of!(HandleInformation, creation), 4);
    assert_eq!(offset_of!(HandleInformation, access), 12);
    assert_eq!(offset_of!(HandleInformation, write), 20);
    assert_eq!(offset_of!(HandleInformation, volume_serial), 28);
    assert_eq!(offset_of!(HandleInformation, index_low), 48);
    assert_eq!(size_of::<BasicInformation>(), 40);
    assert_eq!(size_of::<StandardInformation>(), 24);
    assert_eq!(size_of::<Overlapped>(), 32);
    let dir = Directory::new();
    std::fs::write(dir.0.join("原稿.txt"), b"owned").unwrap();
    let metadata = std::fs::metadata(dir.0.join("原稿.txt")).unwrap();
    let mut descriptors = Descriptors::default();
    let fd = descriptors.open(&dir.path("原稿.txt"), 0, 0).unwrap();
    let handle = descriptors.handle(fd).unwrap();
    let mut information = HandleInformation::default();
    // SAFETY: this owned descriptor is open and the correctly sized struct is writable.
    let queried = unsafe { GetFileInformationByHandle(handle, &mut information) };
    assert_ne!(queried, 0);
    let ticks = |value: &FileTime| u64::from(value.low) | (u64::from(value.high) << 32);
    assert_eq!(ticks(&information.creation), metadata.creation_time());
    assert_eq!(ticks(&information.write), metadata.last_write_time());
    assert_eq!(u64::from(information.size_low), metadata.file_size());
    let reported = fstat(handle).unwrap();
    assert_eq!(reported.size, metadata.len());
    assert_eq!(reported.dev, u64::from(information.volume_serial));
    assert_eq!(
        reported.ino,
        (u64::from(information.index_high) << 32) | u64::from(information.index_low)
    );
    assert_eq!(reported.mode & S_IFMT, S_IFREG);
    assert_eq!(stat(&dir.path("原稿.txt"), false).unwrap(), reported);
    descriptors.close(fd).unwrap();
}

#[test]
fn descriptors_reuse_numbers_and_release_owned_handles_on_drop() {
    let dir = Directory::new();
    let mut descriptors = Descriptors::default();
    let first = descriptors
        .open(&dir.path("a"), O_CREAT | O_RDWR, 0o666)
        .unwrap();
    let second = descriptors
        .open(&dir.path("b"), O_CREAT | O_RDWR, 0o666)
        .unwrap();
    assert_eq!((first, second), (3, 4));
    assert!(format!("{descriptors:?}").starts_with("Descriptors("));
    descriptors.close(first).unwrap();
    assert_eq!(descriptors.close(first), Err(UV_EBADF));
    assert_eq!(descriptors.handle(-1), Err(UV_EBADF));
    assert_eq!(descriptors.handle(999), Err(UV_EBADF));
    assert_eq!(descriptors.close(-1), Err(UV_EBADF));
    let reused = descriptors.open(&dir.path("a"), O_RDWR, 0).unwrap();
    assert_eq!(reused, first);
    let owned = descriptors.handle(second).unwrap();
    drop(descriptors);
    // SAFETY: GetFileType accepts invalid handles, returning FILE_TYPE_UNKNOWN.
    assert_eq!(unsafe { GetFileType(owned) }, 0);
    assert_eq!(last_error(), ERROR_INVALID_HANDLE);
}

#[test]
fn owned_file_operations_preserve_positions_metadata_and_failures() {
    let dir = Directory::new();
    let path = dir.path("data");
    let mut descriptors = Descriptors::default();
    let fd = descriptors
        .open(&path, O_CREAT | O_RDWR | O_EXCL, 0o666)
        .unwrap();
    let handle = descriptors.handle(fd).unwrap();
    let mut first = *b"abc";
    let mut second = *b"def";
    assert_eq!(write(handle, &mut [&mut first, &mut second], -1), Ok(6));
    let mut bytes = [0; 2];
    assert_eq!(read(handle, &mut [&mut bytes], 1), Ok(2));
    assert_eq!(&bytes, b"bc");
    let mut suffix = *b"g";
    assert_eq!(write(handle, &mut [&mut suffix], -1), Ok(1));
    assert_eq!(std::fs::read(dir.0.join("data")).unwrap(), b"abcdefg");
    assert_eq!(read(handle, &mut [&mut bytes], -1), Ok(0));
    fsync(handle).unwrap();
    ftruncate(handle, 3).unwrap();
    assert_eq!(std::fs::read(dir.0.join("data")).unwrap(), b"abc");
    futime(handle, 1_700_000_000.0, 1_700_000_001.0).unwrap();
    assert_eq!(fstat(handle).unwrap().times[1], (1_700_000_001, 0));
    fchmod(handle, 0o444).unwrap();
    assert_eq!(access(&path, W_OK), Err(UV_EPERM));
    fchmod(handle, 0o666).unwrap();
    assert_eq!(access(&path, W_OK), Ok(()));
    descriptors.close(fd).unwrap();
    chmod(&path, 0o444).unwrap();
    assert_eq!(access(&path, 0), Ok(()));
    chmod(&path, 0o666).unwrap();
    utime(&path, 1_700_000_002.0, 1_700_000_003.0, false).unwrap();
    assert_eq!(stat(&path, true).unwrap().times[1], (1_700_000_003, 0));
    let copy = dir.path("copy");
    copyfile(&path, &copy, COPYFILE_EXCL).unwrap();
    assert!(copyfile(&path, &copy, COPYFILE_EXCL).is_err());
    assert_eq!(
        copyfile(&path, &copy, COPYFILE_FICLONE_FORCE),
        Err(UV_ENOSYS)
    );
    copyfile(&path, &path, 0).unwrap();
    let renamed = dir.path("renamed");
    rename(&copy, &renamed).unwrap();
    let linked = dir.path("hard");
    link(&path, &linked).unwrap();
    assert_eq!(
        stat(&path, false).unwrap().ino,
        stat(&linked, false).unwrap().ino
    );
    assert_eq!(stat(&path, false).unwrap().nlink, 2);
    remove(&linked, false).unwrap();
    remove(&renamed, false).unwrap();
    let folder = dir.path("folder");
    mkdir(&folder).unwrap();
    assert!(mkdir(&folder).is_err());
    assert_eq!(stat(&folder, false).unwrap().mode & S_IFMT, S_IFDIR);
    assert_eq!(access(&folder, W_OK), Ok(()));
    assert!(scandir(&folder).unwrap().is_empty());
    let entries = scandir(&dir.path("")).unwrap();
    assert!(
        entries
            .iter()
            .any(|(name, kind)| String::from_utf16_lossy(name) == "data" && *kind == DIRENT_FILE)
    );
    assert!(
        entries
            .iter()
            .any(|(name, kind)| String::from_utf16_lossy(name) == "folder" && *kind == DIRENT_DIR)
    );
    let canonical = std::fs::canonicalize(dir.0.join("data")).unwrap();
    let canonical = canonical.to_string_lossy();
    let expected: Vec<u16> = canonical
        .strip_prefix(r"\\?\")
        .unwrap_or(&canonical)
        .encode_utf16()
        .collect();
    assert_eq!(realpath(&path).unwrap(), expected);
    assert_eq!(readlink(&path), Err(UV_EINVAL));
    let space = statfs(&path).unwrap();
    assert!(space.bsize > 0 && space.blocks > 0 && space.bfree <= space.blocks);
    remove(&folder, true).unwrap();
    assert!(remove(&folder, true).is_err());
    let missing = dir.path("missing");
    assert_eq!(stat(&missing, false), Err(UV_ENOENT));
    assert_eq!(access(&missing, 0), Err(UV_ENOENT));
    assert_eq!(chmod(&missing, 0o666), Err(UV_ENOENT));
    assert_eq!(utime(&missing, 0.0, 0.0, false), Err(UV_ENOENT));
    assert_eq!(realpath(&missing), Err(UV_ENOENT));
    assert_eq!(readlink(&missing), Err(UV_ENOENT));
    assert!(copyfile(&missing, &copy, 0).is_err());
    assert!(rename(&missing, &copy).is_err());
    assert!(link(&missing, &copy).is_err());
    assert!(fsync(INVALID_HANDLE).is_err());
    assert!(ftruncate(INVALID_HANDLE, 0).is_err());
    assert!(futime(INVALID_HANDLE, 0.0, 0.0).is_err());
    assert!(fchmod(INVALID_HANDLE, 0o666).is_err());
    assert!(fstat(INVALID_HANDLE).is_err());
    assert!(read(INVALID_HANDLE, &mut [&mut bytes], -1).is_err());
    assert!(write(INVALID_HANDLE, &mut [&mut bytes], -1).is_err());
    assert_eq!(timespec(116_444_736_000_000_000 - 1), (-1, 999_999_900));
}

#[test]
fn invalid_path_buffers_never_reach_windows_string_apis() {
    let dir = Directory::new();
    let valid = dir.path("target");
    for mut invalid in [vec![], vec![65], vec![65, 0, 66, 0]] {
        assert_eq!(Descriptors::default().open(&invalid, 0, 0), Err(UV_EINVAL));
        assert_eq!(Directories::default().open(&invalid), Err(UV_EINVAL));
        assert_eq!(remove(&invalid, false), Err(UV_EINVAL));
        assert_eq!(mkdir(&invalid), Err(UV_EINVAL));
        assert_eq!(mkdtemp(&mut invalid), Err(UV_EINVAL));
        assert_eq!(scandir(&invalid), Err(UV_EINVAL));
        assert_eq!(stat(&invalid, false), Err(UV_EINVAL));
        assert_eq!(access(&invalid, 0), Err(UV_EINVAL));
        assert_eq!(chmod(&invalid, 0o666), Err(UV_EINVAL));
        assert_eq!(utime(&invalid, 0.0, 0.0, false), Err(UV_EINVAL));
        assert_eq!(readlink(&invalid), Err(UV_EINVAL));
        assert_eq!(realpath(&invalid), Err(UV_EINVAL));
        assert_eq!(statfs(&invalid), Err(UV_EINVAL));
        for (from, to) in [(&invalid, &valid), (&valid, &invalid)] {
            assert_eq!(rename(from, to), Err(UV_EINVAL));
            assert_eq!(copyfile(from, to, 0), Err(UV_EINVAL));
            assert_eq!(link(from, to), Err(UV_EINVAL));
            assert_eq!(symlink(from, to, 0), Err(UV_EINVAL));
        }
    }
}

#[test]
fn directory_cursors_read_batches_reach_eof_and_reuse_native_ids() {
    let dir = Directory::new();
    std::fs::write(dir.0.join("alpha.txt"), b"a").unwrap();
    std::fs::write(dir.0.join("原稿.txt"), b"b").unwrap();
    std::fs::create_dir(dir.0.join("nested")).unwrap();

    let mut directories = Directories::default();
    let id = directories.open(&dir.path("")).unwrap();
    assert_eq!(id, 3);
    assert_eq!(directories.read(id, 0), Err(UV_EINVAL));

    let mut entries = Vec::new();
    while let Some(batch) = directories.read(id, 1).unwrap() {
        assert_eq!(batch.len(), 1);
        entries.extend(batch);
    }
    assert_eq!(directories.read(id, 1).unwrap(), None);
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().any(|(name, kind)| {
        String::from_utf16_lossy(name) == "alpha.txt" && *kind == DIRENT_FILE
    }));
    assert!(entries.iter().any(|(name, kind)| {
        String::from_utf16_lossy(name) == "原稿.txt" && *kind == DIRENT_FILE
    }));
    assert!(
        entries.iter().any(|(name, kind)| {
            String::from_utf16_lossy(name) == "nested" && *kind == DIRENT_DIR
        })
    );

    directories.close(id).unwrap();
    assert_eq!(directories.read(id, 1), Err(UV_EBADF));
    assert_eq!(directories.close(id), Err(UV_EBADF));
    assert_eq!(directories.read(-1, 1), Err(UV_EBADF));
    assert_eq!(directories.read(99, 1), Err(UV_EBADF));
    let reused = directories.open(&dir.path("nested")).unwrap();
    assert_eq!(reused, id);
    assert_eq!(directories.read(reused, 1).unwrap(), None);
    directories.close(reused).unwrap();
}

#[test]
fn directory_cursor_rejects_files_and_missing_paths() {
    let dir = Directory::new();
    std::fs::write(dir.0.join("file"), b"x").unwrap();
    let mut directories = Directories::default();
    assert_eq!(directories.open(&dir.path("file")), Err(UV_ENOTDIR));
    assert_eq!(directories.open(&dir.path("missing")), Err(UV_ENOENT));
}

#[test]
fn directory_cursor_maps_open_and_read_failures() {
    let dir = Directory::new();
    {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetFileInformationByHandle,
            unsafe extern "system" fn(Handle, *mut HandleInformation) -> i32
        );
        query.expect().once().returning(|_, _| 0);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().once().returning(|| ERROR_ACCESS_DENIED);
        assert_eq!(Directories::default().open(&dir.path("")), Err(UV_EPERM));
    }

    for (error, expected) in [
        (ERROR_INVALID_PARAMETER, UV_ENOTDIR),
        (ERROR_ACCESS_DENIED, UV_EPERM),
    ] {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetFileInformationByHandleEx,
            unsafe extern "system" fn(Handle, u32, *mut c_void, u32) -> i32
        );
        query.expect().once().returning(|_, _, _, _| 0);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().once().returning(move || error);
        let mut directory = super::Directory {
            handle: INVALID_HANDLE,
            pending: VecDeque::new(),
            first: true,
            ended: false,
        };
        assert_eq!(directory.read(1), Err(expected));
    }
}

#[test]
fn directory_cursor_records_trim_names_skip_dots_and_classify_entries() {
    let mut records = Vec::new();
    for (index, (name, attributes)) in [
        ("", 0),
        (".", 0),
        ("..", 0),
        ("device", FILE_ATTRIBUTE_DEVICE),
        ("link", FILE_ATTRIBUTE_REPARSE_POINT),
        ("folder", FILE_ATTRIBUTE_DIRECTORY),
        ("file\0", 0),
    ]
    .into_iter()
    .enumerate()
    {
        let units: Vec<u16> = name.encode_utf16().collect();
        let size = (68 + units.len() * 2).next_multiple_of(8);
        let mut record = vec![0; size];
        if index != 6 {
            record[..4].copy_from_slice(&(size as u32).to_le_bytes());
        }
        record[56..60].copy_from_slice(&attributes.to_le_bytes());
        record[60..64].copy_from_slice(&((units.len() * 2) as u32).to_le_bytes());
        for (index, unit) in units.iter().enumerate() {
            record[68 + index * 2..70 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        records.extend(record);
    }
    let mut session = shimforge::Session::new();
    let query = shimforge::mock!(
        session,
        GetFileInformationByHandleEx,
        unsafe extern "system" fn(Handle, u32, *mut c_void, u32) -> i32
    );
    query
        .expect()
        .once()
        .returning(move |_, class, buffer, size| {
            assert_eq!(class, 14);
            assert!(size as usize >= records.len());
            // SAFETY: read_batch allocated a writable buffer of `size` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(records.as_ptr(), buffer.cast(), records.len())
            };
            1
        });
    query.expect().once().returning(|_, _, _, _| 0);
    let last = shimforge::mock!(session, last_error, fn() -> u32);
    last.expect().returning(|| ERROR_NO_MORE_FILES);
    let mut directory = super::Directory {
        handle: INVALID_HANDLE,
        pending: VecDeque::new(),
        first: true,
        ended: false,
    };
    assert_eq!(
        directory.read(8).unwrap().unwrap(),
        [
            ("device".encode_utf16().collect(), DIRENT_CHAR),
            ("link".encode_utf16().collect(), DIRENT_LINK),
            ("folder".encode_utf16().collect(), DIRENT_DIR),
            ("file".encode_utf16().collect(), DIRENT_FILE),
        ]
    );
}

#[test]
fn open_flags_temporary_directories_and_junctions_follow_libuv() {
    let dir = Directory::new();
    let mut descriptors = Descriptors::default();
    let path = dir.path("flags");
    for flags in [
        O_WRONLY | O_RDWR,
        O_RANDOM | O_SEQUENTIAL,
        O_WRONLY | O_APPEND | O_DIRECT,
        O_RDWR | O_DSYNC | O_SYNC,
    ] {
        assert_eq!(descriptors.open(&path, flags, 0o666), Err(UV_EINVAL));
    }
    assert_eq!(descriptors.open(&path, 0, 0), Err(UV_ENOENT));
    let fd = descriptors
        .open(&path, O_CREAT | O_RDWR | O_EXLOCK, 0o666)
        .unwrap();
    assert!(descriptors.open(&path, O_RDWR, 0).is_err());
    descriptors.close(fd).unwrap();
    for flags in [
        O_RDWR | O_CREAT | O_TRUNC,
        O_RDWR | O_CREAT,
        O_RDWR | O_EXCL,
        O_RDWR | O_SEQUENTIAL,
        O_RDWR | O_RANDOM,
        O_RDWR | O_DSYNC,
        O_RDWR | O_SYNC,
        O_RDWR | O_DIRECT,
        O_DIRECT,
    ] {
        let fd = descriptors
            .open(&path, flags, 0o666)
            .unwrap_or_else(|error| panic!("flags {flags:#x}: {error}"));
        descriptors.close(fd).unwrap();
    }
    // Node 22's FILE_GENERIC_WRITE access does not satisfy Windows'
    // GENERIC_WRITE requirement for TRUNCATE_EXISTING without O_CREAT.
    for flags in [O_RDWR | O_TRUNC, O_RDWR | O_TRUNC | O_EXCL] {
        assert_eq!(descriptors.open(&path, flags, 0o666), Err(UV_EINVAL));
    }
    assert!(descriptors.open(&path, O_CREAT | O_EXCL, 0o666).is_err());
    let temporary = dir.path("temporary");
    let fd = descriptors
        .open(
            &temporary,
            O_CREAT | O_RDWR | O_TEMPORARY | O_SHORT_LIVED,
            0o666,
        )
        .unwrap();
    assert!(stat(&temporary, false).is_ok());
    descriptors.close(fd).unwrap();
    assert_eq!(stat(&temporary, false), Err(UV_ENOENT));
    let readonly = dir.path("readonly");
    let fd = descriptors
        .open(&readonly, O_CREAT | O_RDWR, 0o444)
        .unwrap();
    descriptors.close(fd).unwrap();
    assert_eq!(access(&readonly, W_OK), Err(UV_EPERM));
    chmod(&readonly, 0o666).unwrap();
    let mut template = dir.path("prefix-XXXXXX");
    mkdtemp(&mut template).unwrap();
    assert_eq!(stat(&template, false).unwrap().mode & S_IFMT, S_IFDIR);
    remove(&template, true).unwrap();
    assert_eq!(mkdtemp(&mut dir.path("wrong")), Err(UV_EINVAL));
    let mut missing_parent = dir.path("missing/temp-XXXXXX");
    assert_eq!(mkdtemp(&mut missing_parent), Err(UV_ENOENT));

    let target = dir.path("target");
    mkdir(&target).unwrap();
    std::fs::write(dir.0.join("target/owned"), b"keep").unwrap();
    let junction_path = dir.path("junction");
    symlink(&target, &junction_path, SYMLINK_JUNCTION).unwrap();
    assert_eq!(
        readlink(&junction_path).unwrap(),
        target[..target.len() - 1]
    );
    assert_eq!(stat(&junction_path, true).unwrap().mode & S_IFMT, S_IFLNK);
    assert_eq!(
        stat(&junction_path, false).unwrap().ino,
        stat(&target, false).unwrap().ino
    );
    assert_eq!(
        String::from_utf16_lossy(&realpath(&junction_path).unwrap()),
        std::fs::canonicalize(dir.0.join("target"))
            .unwrap()
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .unwrap()
    );
    remove(&junction_path, false).unwrap();
    assert_eq!(std::fs::read(dir.0.join("target/owned")).unwrap(), b"keep");
    assert_eq!(
        symlink(&[65, 0], &dir.path("relative"), SYMLINK_JUNCTION),
        Err(UV_EINVAL)
    );
    assert!(symlink(&target, &dir.path("missing/junction"), SYMLINK_JUNCTION).is_err());
    let symbolic = dir.path("symbolic");
    match symlink(&path, &symbolic, 0) {
        Ok(()) => {
            assert_eq!(readlink(&symbolic).unwrap(), path[..path.len() - 1]);
            assert_eq!(stat(&symbolic, true).unwrap().mode & S_IFMT, S_IFLNK);
            remove(&symbolic, false).unwrap();
        }
        // Developer Mode or the symlink privilege may be unavailable.
        Err(error) => assert_eq!(error, UV_EPERM),
    }
}

#[test]
fn unicode_junction_metadata_counts_wtf8_bytes() {
    let dir = Directory::new();
    let target = dir.path("target原稿🙂");
    mkdir(&target).unwrap();
    let path = dir.path("junction");
    symlink(&target, &path, SYMLINK_JUNCTION).unwrap();
    let resolved = readlink(&path).unwrap();
    assert_eq!(resolved, target[..target.len() - 1]);
    let bytes = String::from_utf16(&resolved).unwrap().len() as u64;
    assert!(bytes > resolved.len() as u64);
    assert_eq!(stat(&path, true).unwrap().size, bytes);
    utime(&path, 1.0, 2.0, true).unwrap();
    remove(&path, false).unwrap();
    assert_eq!(stat(&target, false).unwrap().mode & S_IFMT, S_IFDIR);
}

fn reparse(tag: u32, base: usize, target: &[u16]) -> Vec<u8> {
    let mut buffer = vec![0; base];
    buffer[..4].copy_from_slice(&tag.to_le_bytes());
    buffer[10..12].copy_from_slice(&((target.len() * 2) as u16).to_le_bytes());
    for unit in target {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    let length = (buffer.len() - 8) as u16;
    buffer[4..6].copy_from_slice(&length.to_le_bytes());
    buffer
}

#[test]
fn reparse_targets_preserve_libuv_paths_and_reject_truncated_data() {
    for (source, expected) in [
        (r"\??\C:\folder\a", r"C:\folder\a"),
        (r"\??\uNc\server\share\a", r"\\server\share\a"),
        (r"\\?\C:\folder\a", r"\\?\C:\folder\a"),
        (r"folder\a", r"folder\a"),
    ] {
        let target: Vec<u16> = source.encode_utf16().collect();
        let buffer = reparse(0xa000000c, 20, &target);
        assert_eq!(
            reparse_target(&buffer).unwrap(),
            expected.encode_utf16().collect::<Vec<_>>()
        );
        for end in 0..buffer.len() {
            assert_eq!(
                reparse_target(&buffer[..end]),
                Err(ERROR_INVALID_REPARSE_DATA)
            );
        }
    }
    let target: Vec<u16> = r"\??\D:\native".encode_utf16().collect();
    assert_eq!(
        reparse_target(&reparse(0xa0000003, 16, &target)).unwrap(),
        r"D:\native".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(
        reparse_target(&reparse(0xa0000003, 16, &[b'x' as u16])),
        Err(ERROR_SYMLINK_NOT_SUPPORTED)
    );
    assert_eq!(reparse_target(&[0; 8]), Err(ERROR_SYMLINK_NOT_SUPPORTED));
    let mut bad = reparse(0xa000000c, 20, &[0xd800]);
    assert_eq!(reparse_target(&bad).unwrap(), [0xd800]);
    bad[8..10].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(reparse_target(&bad), Err(ERROR_INVALID_REPARSE_DATA));
    bad[8..10].copy_from_slice(&100u16.to_le_bytes());
    assert_eq!(reparse_target(&bad), Err(ERROR_INVALID_REPARSE_DATA));
    bad[8..10].copy_from_slice(&0u16.to_le_bytes());
    bad[10..12].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(reparse_target(&bad), Err(ERROR_INVALID_REPARSE_DATA));

    let mut app = vec![0; 12];
    app[..4].copy_from_slice(&0x8000001bu32.to_le_bytes());
    app[8..12].copy_from_slice(&3u32.to_le_bytes());
    for unit in "package\0app\0C:\\native.exe\0\0".encode_utf16() {
        app.extend_from_slice(&unit.to_le_bytes());
    }
    let length = (app.len() - 8) as u16;
    app[4..6].copy_from_slice(&length.to_le_bytes());
    assert_eq!(
        reparse_target(&app).unwrap(),
        r"C:\native.exe".encode_utf16().collect::<Vec<_>>()
    );
    app.truncate(app.len() - 4);
    let length = (app.len() - 8) as u16;
    app[4..6].copy_from_slice(&length.to_le_bytes());
    assert_eq!(reparse_target(&app), Err(ERROR_INVALID_REPARSE_DATA));
}

#[test]
fn standard_handles_and_device_statistics_follow_windows_types() {
    let mut descriptors = Descriptors::default();
    assert_eq!(format!("{descriptors:?}"), "Descriptors(0)");
    for fd in 0..=2 {
        descriptors.close(fd).unwrap();
    }
    {
        let mut session = shimforge::Session::new();
        let standard = shimforge::mock!(
            session,
            GetStdHandle,
            unsafe extern "system" fn(u32) -> Handle
        );
        standard.expect().returning(|_| core::ptr::null_mut());
        for fd in 0..=2 {
            assert_eq!(descriptors.handle(fd), Err(UV_EBADF));
        }
    }
    {
        let mut session = shimforge::Session::new();
        let standard = shimforge::mock!(
            session,
            GetStdHandle,
            unsafe extern "system" fn(u32) -> Handle
        );
        standard.expect().returning(|_| INVALID_HANDLE);
        assert_eq!(descriptors.handle(1), Err(UV_EBADF));
    }
    {
        let mut session = shimforge::Session::new();
        let standard = shimforge::mock!(
            session,
            GetStdHandle,
            unsafe extern "system" fn(u32) -> Handle
        );
        standard.expect().returning(|which| {
            assert_eq!(which, -11i32 as u32);
            19usize as Handle
        });
        assert_eq!(descriptors.handle(1), Ok(19usize as Handle));
    }
    let nul: Vec<u16> = "NUL\0".encode_utf16().collect();
    let fd = descriptors.open(&nul, O_RDWR, 0).unwrap();
    let handle = descriptors.handle(fd).unwrap();
    assert_eq!(fstat(handle).unwrap().mode, S_IFCHR | 0o666);
    descriptors.close(fd).unwrap();
    {
        let mut session = shimforge::Session::new();
        let kind = shimforge::mock!(
            session,
            GetFileType,
            unsafe extern "system" fn(Handle) -> u32
        );
        kind.expect().returning(|_| 3);
        assert_eq!(
            fstat(19usize as Handle).unwrap(),
            device_stat(S_IFIFO, 0x11, 19)
        );
    }
    {
        let mut session = shimforge::Session::new();
        let kind = shimforge::mock!(
            session,
            GetFileType,
            unsafe extern "system" fn(Handle) -> u32
        );
        kind.expect().returning(|_| 2);
        let console = shimforge::mock!(
            session,
            GetConsoleMode,
            unsafe extern "system" fn(Handle, *mut u32) -> i32
        );
        console.expect().returning(|_, _| 1);
        assert_eq!(
            fstat(19usize as Handle).unwrap(),
            device_stat(S_IFCHR, 0x50, 19)
        );
    }
}

#[test]
fn deletion_fallback_preserves_attributes_and_reports_kernel_failures() {
    let dir = Directory::new();
    let path = dir.path("owned");
    std::fs::write(dir.0.join("owned"), b"keep").unwrap();
    let mut descriptors = Descriptors::default();
    let fd = descriptors.open(&path, O_RDWR, 0).unwrap();
    let handle = descriptors.handle(fd).unwrap();
    assert_eq!(remove_handle(INVALID_HANDLE, false), Err(UV_EBADF));
    assert_eq!(remove_handle(handle, true), Err(UV_ENOENT));
    for error in [
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_PARAMETER,
        ERROR_INVALID_FUNCTION,
    ] {
        let mut session = shimforge::Session::new();
        let set = shimforge::mock!(
            session,
            set_information,
            fn(Handle, u32, &[u8]) -> Result<(), u32>
        );
        set.expect().once().returning(move |_, class, data| {
            assert_eq!(class, 21);
            assert_eq!(data, [0x13, 0, 0, 0]);
            Err(error)
        });
        set.expect().once().returning(|_, class, data| {
            assert_eq!(class, 4);
            assert_eq!(data, [1]);
            Ok(())
        });
        remove_handle(handle, false).unwrap();
    }
    {
        let mut session = shimforge::Session::new();
        let set = shimforge::mock!(
            session,
            set_information,
            fn(Handle, u32, &[u8]) -> Result<(), u32>
        );
        set.expect().returning(|_, _, _| Err(ERROR_ACCESS_DENIED));
        assert_eq!(remove_handle(handle, false), Err(UV_EPERM));
    }
    chmod(&path, 0o444).unwrap();
    {
        let mut session = shimforge::Session::new();
        let set = shimforge::mock!(
            session,
            set_information,
            fn(Handle, u32, &[u8]) -> Result<(), u32>
        );
        set.expect().once().returning(|_, class, _| {
            assert_eq!(class, 21);
            Err(ERROR_NOT_SUPPORTED)
        });
        set.expect().once().returning(|_, class, bytes| {
            assert_eq!(class, 0);
            let attributes = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
            assert_eq!(attributes & FILE_ATTRIBUTE_READONLY, 0);
            assert_ne!(attributes & FILE_ATTRIBUTE_ARCHIVE, 0);
            Ok(())
        });
        set.expect().once().returning(|_, class, _| {
            assert_eq!(class, 4);
            Err(ERROR_ACCESS_DENIED)
        });
        assert_eq!(remove_handle(handle, false), Err(UV_EPERM));
    }
    chmod(&path, 0o666).unwrap();
    descriptors.close(fd).unwrap();
    mkdir(&dir.path("folder")).unwrap();
    assert_eq!(remove(&dir.path("folder"), false), Err(UV_EPERM));
    std::fs::write(dir.0.join("folder/child"), b"keep").unwrap();
    assert!(remove(&dir.path("folder"), true).is_err());
}

#[test]
fn transfer_errors_distinguish_permissions_eof_and_broken_pipes() {
    for (error, read_result, write_result) in [
        (ERROR_ACCESS_DENIED, Err(UV_EBADF), Err(UV_EBADF)),
        (ERROR_BROKEN_PIPE, Ok(0), Err(UV_EPIPE)),
        (ERROR_HANDLE_EOF, Ok(0), Err(translate(ERROR_HANDLE_EOF))),
        (ERROR_NO_DATA, Err(translate(ERROR_NO_DATA)), Err(UV_EPIPE)),
        (ERROR_INVALID_HANDLE, Err(UV_EBADF), Err(UV_EBADF)),
    ] {
        let mut session = shimforge::Session::new();
        let transfer_mock = shimforge::mock!(
            session,
            transfer,
            fn(Handle, i64, &mut [&mut [u8]], bool) -> (usize, bool)
        );
        transfer_mock.expect().returning(|_, _, _, _| (0, false));
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(move || error);
        assert_eq!(read(INVALID_HANDLE, &mut [&mut [0]], -1), read_result);
        assert_eq!(write(INVALID_HANDLE, &mut [&mut [0]], -1), write_result);
    }
    let mut session = shimforge::Session::new();
    let transfer_mock = shimforge::mock!(
        session,
        transfer,
        fn(Handle, i64, &mut [&mut [u8]], bool) -> (usize, bool)
    );
    transfer_mock.expect().returning(|_, _, _, _| (2, false));
    assert_eq!(read(INVALID_HANDLE, &mut [], -1), Ok(2));
    assert_eq!(write(INVALID_HANDLE, &mut [], -1), Ok(2));
}

#[test]
fn permission_updates_restore_archive_state_and_surface_write_failures() {
    {
        let mut session = shimforge::Session::new();
        let basic = shimforge::mock!(
            session,
            query_basic,
            fn(Handle) -> Result<BasicInformation, u32>
        );
        basic
            .expect()
            .returning(|_| Ok(BasicInformation::default()));
        let set = shimforge::mock!(
            session,
            set_information,
            fn(Handle, u32, &[u8]) -> Result<(), u32>
        );
        for expected in [
            FILE_ATTRIBUTE_ARCHIVE,
            FILE_ATTRIBUTE_ARCHIVE,
            FILE_ATTRIBUTE_NORMAL,
        ] {
            set.expect().once().returning(move |_, class, bytes| {
                assert_eq!(class, 0);
                assert_eq!(
                    u32::from_le_bytes(bytes[32..36].try_into().unwrap()),
                    expected
                );
                Ok(())
            });
        }
        fchmod(INVALID_HANDLE, 0o666).unwrap();
    }
    {
        let mut session = shimforge::Session::new();
        let basic = shimforge::mock!(
            session,
            query_basic,
            fn(Handle) -> Result<BasicInformation, u32>
        );
        basic.expect().returning(|_| {
            Ok(BasicInformation {
                attributes: FILE_ATTRIBUTE_NORMAL,
                ..BasicInformation::default()
            })
        });
        let set = shimforge::mock!(
            session,
            set_information,
            fn(Handle, u32, &[u8]) -> Result<(), u32>
        );
        set.expect().once().returning(|_, _, _| Ok(()));
        set.expect().once().returning(|_, _, _| Ok(()));
        set.expect().once().returning(|_, _, bytes| {
            assert_ne!(
                u32::from_le_bytes(bytes[32..36].try_into().unwrap()) & FILE_ATTRIBUTE_READONLY,
                0
            );
            Ok(())
        });
        fchmod(INVALID_HANDLE, 0o444).unwrap();
    }
    let dir = Directory::new();
    let path = dir.path("owned");
    std::fs::write(dir.0.join("owned"), b"keep").unwrap();
    let mut session = shimforge::Session::new();
    let set = shimforge::mock!(
        session,
        SetFileAttributesW,
        unsafe extern "system" fn(*const u16, u32) -> i32
    );
    set.expect().returning(|_, _| 0);
    let last = shimforge::mock!(session, last_error, fn() -> u32);
    last.expect().returning(|| ERROR_ACCESS_DENIED);
    assert_eq!(chmod(&path, 0o444), Err(UV_EPERM));
}

#[test]
fn temporary_directory_collision_retries_are_bounded() {
    let dir = Directory::new();
    let mut session = shimforge::Session::new();
    let create = shimforge::mock!(
        session,
        CreateDirectoryW,
        unsafe extern "system" fn(*const u16, *mut c_void) -> i32
    );
    create.expect().times(32_767).returning(|_, _| 0);
    let last = shimforge::mock!(session, last_error, fn() -> u32);
    last.expect().returning(|| ERROR_ALREADY_EXISTS);
    assert_eq!(
        mkdtemp(&mut dir.path("owned-XXXXXX")),
        Err(translate(ERROR_ALREADY_EXISTS))
    );
}

#[test]
fn junction_failures_remove_only_the_new_directory_and_bound_reparse_data() {
    let dir = Directory::new();
    for failure_at_open in [true, false] {
        let path = dir.path("junction");
        let mut session = shimforge::Session::new();
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_ACCESS_DENIED);
        if failure_at_open {
            let open = shimforge::mock!(session, create, fn(&[u16], u32, u32, u32, u32) -> Handle);
            open.expect().returning(|_, _, _, _, _| INVALID_HANDLE);
        } else {
            let control = shimforge::mock!(
                session,
                DeviceIoControl,
                unsafe extern "system" fn(
                    Handle,
                    u32,
                    *const c_void,
                    u32,
                    *mut c_void,
                    u32,
                    *mut u32,
                    *mut c_void,
                ) -> i32
            );
            control.expect().returning(|_, _, _, _, _, _, _, _| 0);
        }
        assert_eq!(
            symlink(&dir.path("target"), &path, SYMLINK_JUNCTION),
            Err(UV_EPERM)
        );
        assert!(!dir.0.join("junction").exists());
    }
    let mut huge: Vec<u16> = "C:\\".encode_utf16().collect();
    huge.extend(core::iter::repeat_n(b'x' as u16, 5000));
    huge.push(0);
    assert_eq!(
        symlink(&huge, &dir.path("huge"), SYMLINK_JUNCTION),
        Err(translate(206))
    );
    assert!(!dir.0.join("huge").exists());
    for target in [r"\\?\C:\", r"C:\\folder\\", r"C:/folder//"] {
        let mut target: Vec<u16> = target.encode_utf16().collect();
        target.push(0);
        let mut session = shimforge::Session::new();
        let create_dir = shimforge::mock!(
            session,
            CreateDirectoryW,
            unsafe extern "system" fn(*const u16, *mut c_void) -> i32
        );
        create_dir.expect().returning(|_, _| 0);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_ALREADY_EXISTS);
        assert_eq!(
            symlink(&target, &dir.path("owned"), SYMLINK_JUNCTION),
            Err(translate(ERROR_ALREADY_EXISTS))
        );
    }
}

#[test]
fn symbolic_link_legacy_retry_preserves_flags_and_reparse_bounds() {
    let dir = Directory::new();
    {
        let mut session = shimforge::Session::new();
        let create = shimforge::mock!(
            session,
            CreateSymbolicLinkW,
            unsafe extern "system" fn(*const u16, *const u16, u32) -> u8
        );
        create.expect().once().returning(|_, _, flags| {
            assert_eq!(flags, 3);
            0
        });
        create.expect().once().returning(|_, _, flags| {
            assert_eq!(flags, 1);
            1
        });
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_INVALID_PARAMETER);
        symlink(&dir.path("target"), &dir.path("owned"), SYMLINK_DIR).unwrap();
    }
    {
        let mut session = shimforge::Session::new();
        let control = shimforge::mock!(
            session,
            DeviceIoControl,
            unsafe extern "system" fn(
                Handle,
                u32,
                *const c_void,
                u32,
                *mut c_void,
                u32,
                *mut u32,
                *mut c_void,
            ) -> i32
        );
        control.expect().returning(|_, _, _, _, _, _, returned, _| {
            // SAFETY: readlink_handle passes a writable u32 output.
            unsafe { *returned = 16 * 1024 + 1 };
            1
        });
        assert_eq!(
            readlink_handle(INVALID_HANDLE),
            Err(ERROR_INVALID_REPARSE_DATA)
        );
    }
}

#[test]
fn final_path_queries_handle_unc_errors_and_concurrent_growth() {
    for (source, expected) in [
        (r"\\?\UNC\server\share\owned", Ok(r"\\server\share\owned")),
        (r"\\?\C:\owned", Ok(r"C:\owned")),
        (r"C:\owned", Err(ERROR_INVALID_HANDLE)),
    ] {
        let units: Vec<u16> = source.encode_utf16().collect();
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetFinalPathNameByHandleW,
            unsafe extern "system" fn(Handle, *mut u16, u32, u32) -> u32
        );
        query.expect().returning(move |_, path, length, _| {
            if path.is_null() {
                return units.len() as u32 + 1;
            }
            assert!(length as usize > units.len());
            // SAFETY: final_path allocated at least length writable units.
            unsafe { core::ptr::copy_nonoverlapping(units.as_ptr(), path, units.len()) };
            units.len() as u32
        });
        assert_eq!(
            final_path(INVALID_HANDLE),
            expected.map(|path| path.encode_utf16().collect())
        );
    }
    {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetFinalPathNameByHandleW,
            unsafe extern "system" fn(Handle, *mut u16, u32, u32) -> u32
        );
        query.expect().returning(|_, _, _, _| 0);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_ACCESS_DENIED);
        assert_eq!(final_path(INVALID_HANDLE), Err(ERROR_ACCESS_DENIED));
    }
    for written in [0, 20] {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetFinalPathNameByHandleW,
            unsafe extern "system" fn(Handle, *mut u16, u32, u32) -> u32
        );
        query.expect().once().returning(|_, _, _, _| 10);
        query.expect().once().returning(move |_, _, _, _| written);
        assert_eq!(final_path(INVALID_HANDLE), Err(ERROR_INVALID_HANDLE));
    }
}

#[test]
fn volume_queries_reject_failed_and_oversized_directory_expansion() {
    let dir = Directory::new();
    for length in [0, 32_768, 1] {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetDiskFreeSpaceW,
            unsafe extern "system" fn(*const u16, *mut u32, *mut u32, *mut u32, *mut u32) -> i32
        );
        query.expect().returning(|_, _, _, _, _| 0);
        let full = shimforge::mock!(
            session,
            GetFullPathNameW,
            unsafe extern "system" fn(*const u16, u32, *mut u16, *mut *mut u16) -> u32
        );
        full.expect().returning(move |_, _, _, _| length);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_DIRECTORY);
        let expected = if length == 32_768 {
            translate(206)
        } else {
            translate(ERROR_DIRECTORY)
        };
        assert_eq!(statfs(&dir.path("owned")), Err(expected));
    }
}

#[test]
fn directory_records_classify_devices_links_and_skip_dot_entries() {
    let mut records = Vec::new();
    for (index, (name, attributes)) in [
        ("", 0),
        (".", 0),
        ("..", 0),
        ("device", FILE_ATTRIBUTE_DEVICE),
        ("link", FILE_ATTRIBUTE_REPARSE_POINT),
        ("folder", FILE_ATTRIBUTE_DIRECTORY),
        ("file\0", 0),
    ]
    .into_iter()
    .enumerate()
    {
        let units: Vec<u16> = name.encode_utf16().collect();
        let size = (68 + units.len() * 2).next_multiple_of(8);
        let mut record = vec![0; size];
        if index != 6 {
            record[..4].copy_from_slice(&(size as u32).to_le_bytes());
        }
        record[56..60].copy_from_slice(&attributes.to_le_bytes());
        record[60..64].copy_from_slice(&((units.len() * 2) as u32).to_le_bytes());
        for (index, unit) in units.iter().enumerate() {
            record[68 + index * 2..70 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        records.extend(record);
    }
    let mut session = shimforge::Session::new();
    let query = shimforge::mock!(
        session,
        GetFileInformationByHandleEx,
        unsafe extern "system" fn(Handle, u32, *mut c_void, u32) -> i32
    );
    query
        .expect()
        .once()
        .returning(move |_, class, buffer, size| {
            assert_eq!(class, 14);
            assert!(size as usize >= records.len());
            // SAFETY: scan allocated a writable buffer of size bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(records.as_ptr(), buffer.cast(), records.len())
            };
            1
        });
    query.expect().once().returning(|_, _, _, _| 0);
    let last = shimforge::mock!(session, last_error, fn() -> u32);
    last.expect().returning(|| ERROR_NO_MORE_FILES);
    let entries = scan(INVALID_HANDLE).unwrap();
    assert_eq!(
        entries,
        [
            ("device", DIRENT_CHAR),
            ("link", DIRENT_LINK),
            ("folder", DIRENT_DIR),
            ("file", DIRENT_FILE),
        ]
        .map(|(name, kind)| (name.encode_utf16().collect(), kind))
    );
}

#[test]
fn metadata_failures_and_nonlink_reparse_retries_close_handles() {
    let dir = Directory::new();
    let path = dir.path("owned");
    std::fs::write(dir.0.join("owned"), b"keep").unwrap();
    {
        let mut session = shimforge::Session::new();
        let stat_mock =
            shimforge::mock!(session, stat_handle, fn(Handle, bool) -> Result<Stat, u32>);
        stat_mock.expect().once().returning(|_, link| {
            assert!(link);
            Err(ERROR_SYMLINK_NOT_SUPPORTED)
        });
        stat_mock.expect().once().returning(|_, link| {
            assert!(!link);
            Ok(Stat {
                size: 4,
                ..Stat::default()
            })
        });
        assert_eq!(stat(&path, true).unwrap().size, 4);
    }
    {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetFileInformationByHandleEx,
            unsafe extern "system" fn(Handle, u32, *mut c_void, u32) -> i32
        );
        query.expect().returning(|_, class, output, _| {
            if class == 1 {
                return 0;
            }
            assert_eq!(class, 0);
            // SAFETY: stat_handle passes a writable FILE_BASIC_INFO.
            unsafe {
                output
                    .cast::<BasicInformation>()
                    .write(BasicInformation::default())
            };
            1
        });
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_ACCESS_DENIED);
        assert_eq!(stat(&path, false), Err(UV_EPERM));
    }
    // Both paths above close their handles: exclusive reopening succeeds.
    let mut descriptors = Descriptors::default();
    let fd = descriptors.open(&path, O_RDWR | O_EXLOCK, 0).unwrap();
    descriptors.close(fd).unwrap();
    chmod(&path, 0o444).unwrap();
    assert_eq!(stat(&path, false).unwrap().mode & 0o777, 0o444);
    chmod(&path, 0o666).unwrap();
    assert!(stat(&dir.path("."), false).is_ok());
    let mut trailing = dir.path(".");
    trailing.pop();
    trailing.extend([b'/' as u16, 0]);
    assert!(stat(&trailing, false).is_ok());
    assert!(scan(INVALID_HANDLE).is_err());
}

#[test]
fn unlink_refuses_directory_reparse_points_that_are_not_links() {
    for error in [ERROR_SYMLINK_NOT_SUPPORTED, ERROR_ACCESS_DENIED] {
        let mut session = shimforge::Session::new();
        let info = shimforge::mock!(
            session,
            GetFileInformationByHandle,
            unsafe extern "system" fn(Handle, *mut HandleInformation) -> i32
        );
        info.expect().returning(|_, info| {
            // SAFETY: remove_handle passes a writable BY_HANDLE_FILE_INFORMATION.
            unsafe {
                info.write(HandleInformation {
                    attributes: FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
                    ..HandleInformation::default()
                })
            };
            1
        });
        let target = shimforge::mock!(
            session,
            readlink_handle,
            fn(Handle) -> Result<Vec<u16>, u32>
        );
        target.expect().returning(move |_| Err(error));
        assert_eq!(remove_handle(INVALID_HANDLE, false), Err(UV_EPERM));
    }
}

#[test]
fn busy_copy_errors_are_preserved_for_distinct_or_missing_files() {
    let dir = Directory::new();
    std::fs::write(dir.0.join("from"), b"from").unwrap();
    std::fs::write(dir.0.join("to"), b"to").unwrap();
    let mut session = shimforge::Session::new();
    let copy = shimforge::mock!(
        session,
        CopyFileW,
        unsafe extern "system" fn(*const u16, *const u16, i32) -> i32
    );
    copy.expect().returning(|_, _, _| 0);
    let last = shimforge::mock!(session, last_error, fn() -> u32);
    last.expect().returning(|| 32); // ERROR_SHARING_VIOLATION.
    assert_eq!(
        copyfile(&dir.path("from"), &dir.path("to"), 0),
        Err(UV_EBUSY)
    );
    assert_eq!(
        copyfile(&dir.path("missing"), &dir.path("to"), 0),
        Err(UV_EBUSY)
    );
    assert_eq!(std::fs::read(dir.0.join("to")).unwrap(), b"to");
}

#[test]
fn remaining_filesystem_failures_preserve_operation_error_codes() {
    let dir = Directory::new();
    assert_eq!(scandir(&dir.path("missing")), Err(UV_ENOENT));
    assert_eq!(
        stat_handle(INVALID_HANDLE, false),
        Err(ERROR_INVALID_HANDLE)
    );
    assert!(statfs(&dir.path("missing")).is_err());
    {
        let mut session = shimforge::Session::new();
        let open = shimforge::mock!(session, create, fn(&[u16], u32, u32, u32, u32) -> Handle);
        open.expect().returning(|_, _, _, _, _| INVALID_HANDLE);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_FILE_EXISTS);
        assert_eq!(
            open_handle(&dir.path("owned"), O_CREAT, 0o666),
            Err(UV_EISDIR)
        );
    }
    {
        let mut session = shimforge::Session::new();
        let create = shimforge::mock!(
            session,
            CreateDirectoryW,
            unsafe extern "system" fn(*const u16, *mut c_void) -> i32
        );
        create.expect().returning(|_, _| 0);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_INVALID_NAME);
        assert_eq!(mkdir(&dir.path("owned")), Err(UV_EINVAL));
    }
    {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetFileInformationByHandleEx,
            unsafe extern "system" fn(Handle, u32, *mut c_void, u32) -> i32
        );
        query.expect().returning(|_, _, _, _| 0);
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_INVALID_PARAMETER);
        assert_eq!(scan(INVALID_HANDLE), Err(UV_ENOTDIR));
    }
    std::fs::write(dir.0.join("owned"), b"keep").unwrap();
    {
        let mut session = shimforge::Session::new();
        let target = shimforge::mock!(
            session,
            readlink_handle,
            fn(Handle) -> Result<Vec<u16>, u32>
        );
        target.expect().returning(|_| Err(ERROR_ACCESS_DENIED));
        assert_eq!(readlink(&dir.path("owned")), Err(UV_EPERM));
    }
}

#[test]
fn app_execution_aliases_require_three_nonempty_strings_and_an_absolute_target() {
    for (count, names) in [
        (2, "package\0app\0C:\\owned.exe\0"),
        (3, "\0app\0C:\\owned.exe\0"),
        (3, "package\0\0C:\\owned.exe\0"),
        (3, "package\0app\0relative.exe\0"),
        (3, "package\0app\0\0"),
    ] {
        let mut buffer = vec![0; 12];
        buffer[..4].copy_from_slice(&0x8000001bu32.to_le_bytes());
        buffer[8..12].copy_from_slice(&(count as u32).to_le_bytes());
        for unit in names.encode_utf16() {
            buffer.extend_from_slice(&unit.to_le_bytes());
        }
        let length = (buffer.len() - 8) as u16;
        buffer[4..6].copy_from_slice(&length.to_le_bytes());
        assert_eq!(reparse_target(&buffer), Err(ERROR_SYMLINK_NOT_SUPPORTED));
    }
}

#[test]
fn volume_queries_retry_the_owned_files_parent_directory() {
    let dir = Directory::new();
    std::fs::write(dir.0.join("owned"), b"keep").unwrap();
    let root = dir.0.ancestors().last().unwrap();
    use std::os::windows::ffi::OsStrExt;
    let root: Vec<u16> = root.as_os_str().encode_wide().chain([0]).collect();
    assert!(statfs(&root).unwrap().bsize > 0);
    for path in [dir.path("owned"), root] {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(
            session,
            GetDiskFreeSpaceW,
            unsafe extern "system" fn(*const u16, *mut u32, *mut u32, *mut u32, *mut u32) -> i32
        );
        query.expect().once().returning(|_, _, _, _, _| 0);
        query
            .expect()
            .once()
            .returning(|path, sectors, bytes, free, total| {
                let mut length = 0;
                // SAFETY: statfs passes a terminated path and four writable outputs.
                unsafe {
                    while *path.add(length) != 0 {
                        length += 1;
                    }
                    let path =
                        String::from_utf16(core::slice::from_raw_parts(path, length)).unwrap();
                    assert!(!path.ends_with("owned"));
                    *sectors = 2;
                    *bytes = 1024;
                    *free = 3;
                    *total = 4;
                }
                1
            });
        let last = shimforge::mock!(session, last_error, fn() -> u32);
        last.expect().returning(|| ERROR_DIRECTORY);
        assert_eq!(
            statfs(&path),
            Ok(StatFs {
                bsize: 2048,
                blocks: 4,
                bfree: 3
            })
        );
    }
}

#[test]
fn asar_paths_reach_the_folders_their_archives_were_packed_from() {
    let units = |text: &str| text.encode_utf16().collect::<Vec<_>>();
    let text = |units: Vec<u16>| String::from_utf16(&units).unwrap();
    let existing = [
        r"C:\app",
        r"C:\app\node_modules",
        r"C:\app\node_modules\x\a.wasm",
        r"C:\app\real.asar",
    ];
    let exists = |path: &[u16]| existing.iter().any(|known| units(known) == path);
    let map = |path: &str| without_asar_in(&units(path), exists).map(text);
    assert_eq!(
        map(r"C:\app\node_modules.asar.unpacked\x\a.wasm").as_deref(),
        Some(r"C:\app\node_modules\x\a.wasm")
    );
    assert_eq!(
        map(r"C:\app\node_modules.ASAR\x").as_deref(),
        Some(r"C:\app\node_modules\x")
    );
    // Existing archives and paths without them stay as they are.
    assert_eq!(map(r"C:\app\real.asar\x"), None);
    assert_eq!(map(r"C:\app\missing.asar\x"), None);
    assert_eq!(map(r"C:\app\node_modules\x"), None);
    assert_eq!(map(r"C:\app\.asar"), None);
    let directory = Directory::new();
    std::fs::create_dir(directory.0.join("pack")).unwrap();
    let mut path = directory.path("pack.asar.unpacked");
    path.pop();
    let mut expected = directory.path("pack");
    expected.pop();
    assert_eq!(without_asar(&path), Some(expected));
}

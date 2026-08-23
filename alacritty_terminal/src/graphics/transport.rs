use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as Base64;

use super::{Command, GraphicsError, Transmission};

pub fn load_transport(
    transmission: Transmission,
    command: &Command,
    limit: usize,
) -> Result<Vec<u8>, GraphicsError> {
    let name = Base64.decode(&command.payload).map_err(|_| GraphicsError::Invalid)?;
    match transmission {
        Transmission::Direct => Err(GraphicsError::Invalid),
        Transmission::File => read_file(native_path(name), command, limit),
        Transmission::TemporaryFile => {
            let path = native_path(name);
            let data = read_file(path.clone(), command, limit)?;
            if temporary_path_is_safe(&path) {
                let _ = fs::remove_file(path);
            }
            Ok(data)
        },
        Transmission::SharedMemory => read_shared_memory(name, command, limit),
    }
}

#[cfg(unix)]
fn native_path(bytes: Vec<u8>) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn native_path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_file(path: PathBuf, command: &Command, limit: usize) -> Result<Vec<u8>, GraphicsError> {
    if sensitive_path(&path) {
        return Err(GraphicsError::Io);
    }
    let mut file = open_read_only(&path)?;
    read_regular_range(&mut file, command, limit)
}

#[cfg(unix)]
fn open_read_only(path: &Path) -> Result<File, GraphicsError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| GraphicsError::Io)
}

#[cfg(not(unix))]
fn open_read_only(path: &Path) -> Result<File, GraphicsError> {
    OpenOptions::new().read(true).open(path).map_err(|_| GraphicsError::Io)
}

fn read_regular_range(
    file: &mut File,
    command: &Command,
    limit: usize,
) -> Result<Vec<u8>, GraphicsError> {
    let metadata = file.metadata().map_err(|_| GraphicsError::Io)?;
    if !metadata.file_type().is_file() {
        return Err(GraphicsError::Invalid);
    }

    let offset = u64::from(command.data_offset.unwrap_or(0));
    let available = metadata.len().checked_sub(offset).ok_or(GraphicsError::Invalid)?;
    let requested = match command.data_size.unwrap_or(0) {
        0 => available,
        size => u64::from(size),
    };
    if requested > available {
        return Err(GraphicsError::Invalid);
    }
    let requested = usize::try_from(requested).map_err(|_| GraphicsError::TooLarge)?;
    if requested > limit {
        return Err(GraphicsError::NoSpace);
    }

    file.seek(SeekFrom::Start(offset)).map_err(|_| GraphicsError::Io)?;
    let mut data = vec![0; requested];
    file.read_exact(&mut data).map_err(|_| GraphicsError::Io)?;
    Ok(data)
}

fn sensitive_path(path: &Path) -> bool {
    #[cfg(unix)]
    {
        ["/proc", "/sys", "/dev"]
            .iter()
            .any(|prefix| path.starts_with(prefix) && !path.starts_with("/dev/shm"))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

fn temporary_path_is_safe(path: &Path) -> bool {
    if !path.to_string_lossy().contains("tty-graphics-protocol") {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(parent) = parent.canonicalize() else {
        return false;
    };
    temporary_directories()
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .any(|temp| parent.starts_with(temp))
}

fn temporary_directories() -> Vec<PathBuf> {
    let mut directories = vec![std::env::temp_dir()];
    if let Some(path) = std::env::var_os("TMPDIR") {
        directories.push(path.into());
    }
    #[cfg(unix)]
    directories.push(PathBuf::from("/dev/shm"));
    directories
}

#[cfg(unix)]
fn read_shared_memory(
    name: Vec<u8>,
    command: &Command,
    limit: usize,
) -> Result<Vec<u8>, GraphicsError> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;

    let name = CString::new(name).map_err(|_| GraphicsError::Invalid)?;
    // SAFETY: `name` is NUL-terminated and remains live for both libc calls. A successful file
    // descriptor is transferred exactly once into `File`.
    let descriptor = unsafe { libc::shm_open(name.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC, 0) };
    if descriptor < 0 {
        return Err(GraphicsError::Io);
    }
    // SAFETY: `descriptor` is newly owned and valid after successful `shm_open`.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    // POSIX transport lifetime ends once the terminal has opened the object.
    unsafe { libc::shm_unlink(name.as_ptr()) };
    read_regular_range(&mut file, command, limit)
}

#[cfg(not(unix))]
fn read_shared_memory(
    _name: Vec<u8>,
    _command: &Command,
    _limit: usize,
) -> Result<Vec<u8>, GraphicsError> {
    Err(GraphicsError::Unsupported)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    fn command(path: &Path) -> Command {
        #[cfg(unix)]
        let bytes = {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes()
        };
        #[cfg(not(unix))]
        let bytes = path.to_string_lossy().as_bytes();
        Command { payload: Base64.encode(bytes).into_bytes(), ..Default::default() }
    }

    #[test]
    fn reads_bounded_regular_file_range() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&[1, 2, 3, 4]).unwrap();
        let mut command = command(file.path());
        command.data_offset = Some(1);
        command.data_size = Some(2);

        assert_eq!(load_transport(Transmission::File, &command, 2).unwrap(), [2, 3]);
        assert_eq!(load_transport(Transmission::File, &command, 1), Err(GraphicsError::NoSpace));
    }

    #[test]
    fn temporary_file_is_removed_only_with_marker() {
        let directory = tempfile::tempdir().unwrap();
        let safe = directory.path().join("tty-graphics-protocol-image");
        fs::write(&safe, [1, 2, 3, 4]).unwrap();
        load_transport(Transmission::TemporaryFile, &command(&safe), 4).unwrap();
        assert!(!safe.exists());

        let unsafe_path = directory.path().join("image");
        fs::write(&unsafe_path, [1, 2, 3, 4]).unwrap();
        load_transport(Transmission::TemporaryFile, &command(&unsafe_path), 4).unwrap();
        assert!(unsafe_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn reads_and_unlinks_shared_memory() {
        use std::ffi::CString;
        use std::os::fd::FromRawFd;

        let name = format!("/alacritty-kitty-test-{}", std::process::id());
        let c_name = CString::new(name.as_bytes()).unwrap();
        let descriptor = unsafe {
            libc::shm_open(c_name.as_ptr(), libc::O_CREAT | libc::O_EXCL | libc::O_RDWR, 0o600)
        };
        assert!(descriptor >= 0);
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        file.write_all(&[1, 2, 3, 4]).unwrap();
        drop(file);
        let command =
            Command { payload: Base64.encode(name.as_bytes()).into_bytes(), ..Default::default() };

        assert_eq!(load_transport(Transmission::SharedMemory, &command, 4).unwrap(), [1, 2, 3, 4]);
        assert_eq!(unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0) }, -1);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_fifo_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("fifo");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(
            load_transport(Transmission::File, &command(&fifo), 4),
            Err(GraphicsError::Invalid)
        );
    }
}

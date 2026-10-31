//! Waiting for a message to arrive in the queue, rather than looking every
//! few seconds: an inotify watch on `new/`, where every message lands by a
//! rename. Elsewhere, or if the watch cannot be made, waiting is sleeping.

use std::{path::Path, time::Duration};

pub struct Watch {
    #[cfg(target_os = "linux")]
    fd: Option<std::os::fd::OwnedFd>,
}

impl Watch {
    /// Watch `dir` for files arriving. Never fails: without a watch,
    /// [`Watch::wait`] sleeps out its time.
    pub fn new(dir: &Path) -> Watch {
        #[cfg(target_os = "linux")]
        {
            let fd = linux::watch(dir)
                .map_err(|e| log::warn!("not watching {}: {e}; looking every poll", dir.display()))
                .ok();
            Watch { fd }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = dir;
            Watch {}
        }
    }

    /// Whether arrivals wake [`Watch::wait`] (rather than only its time
    /// running out).
    pub fn watching(&self) -> bool {
        #[cfg(target_os = "linux")]
        return self.fd.is_some();
        #[cfg(not(target_os = "linux"))]
        false
    }

    /// Wait until something arrives or `timeout` passes. Returns whether
    /// something arrived.
    pub fn wait(&self, timeout: Duration) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(fd) = &self.fd {
            return linux::wait(fd, timeout);
        }
        std::thread::sleep(timeout);
        false
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        io,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::ffi::OsStrExt,
        },
        path::Path,
        time::Duration,
    };

    pub fn watch(dir: &Path) -> io::Result<OwnedFd> {
        // SAFETY: plain syscalls; the descriptor is owned from here on
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes())?;
        // a message arrives by a rename from tmp/; one written in place
        // (by hand, or another tool) is finished when it is closed
        let mask = libc::IN_MOVED_TO | libc::IN_CLOSE_WRITE;
        // SAFETY: `path` is a valid C string for the call
        if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), path.as_ptr(), mask) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(fd)
    }

    pub fn wait(fd: &OwnedFd, timeout: Duration) -> bool {
        let mut p = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd
        let n = unsafe { libc::poll(&mut p, 1, ms) };
        if n <= 0 {
            return false;
        }
        // the events only say "look again": drain them
        let mut buf = [0u8; 4096];
        // SAFETY: reading into a buffer of the length given
        while unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arrival_wakes_the_wait() {
        let d = tempfile::tempdir().unwrap();
        let w = Watch::new(d.path());
        if !w.watching() {
            return;
        }
        assert!(!w.wait(Duration::from_millis(10)));
        let dir = d.path().to_owned();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            std::fs::write(dir.join("m"), b"x").unwrap();
        });
        let start = std::time::Instant::now();
        assert!(w.wait(Duration::from_secs(10)));
        assert!(start.elapsed() < Duration::from_secs(5));
        t.join().unwrap();
    }
}

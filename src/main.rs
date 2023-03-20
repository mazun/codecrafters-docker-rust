use anyhow::{Context, Result};
use libc::{strerror, unshare, CLONE_NEWNS, CLONE_NEWPID};
use std::{env, ffi::CStr, fs, os::unix::fs::chroot};
use tempfile::TempDir;

fn check_libc_return_code(libc_func: fn() -> i32) -> Result<()> {
    if libc_func() != 0 {
        unsafe {
            let err = *libc::__errno_location();
            let err_str = CStr::from_ptr(strerror(err));
            Err(anyhow::anyhow!(
                "Failed to call unshare: {}",
                err_str.to_string_lossy()
            ))
        }
    } else {
        Ok(())
    }
}

// Usage: your_docker.sh run <image> <command> <arg1> <arg2> ...
fn main() -> Result<()> {
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    // println!("Logs from your program will appear here!");

    let args: Vec<_> = std::env::args().collect();
    let command = &args[3];
    let command_args = &args[4..];

    check_libc_return_code(|| {
        let flags = CLONE_NEWPID | CLONE_NEWNS;
        unsafe { unshare(flags) }
    })?;

    let tmp_dir = TempDir::new()?;
    let new_command = tmp_dir.path().join(command.trim_start_matches('/'));
    fs::create_dir_all(new_command.parent().unwrap())?;
    fs::copy(command, new_command)?;

    chroot(tmp_dir.path())?;
    env::set_current_dir("/")?;

    fs::create_dir_all("/dev")?;
    fs::File::create("/dev/null")?;

    // Somehow, this doesn't work with my docker environment but does with the codecrafters' :).
    // Guess this is related to the .so files used from the binaries like `ls`, but not sure...
    // Another fact is that codecrafters's tests pass even without the preceeding "/". So
    // guess the binary with everything statistically linked is located on the directory the tests run.
    let output = std::process::Command::new("/".to_owned() + command.trim_start_matches('/'))
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .args(command_args)
        .output()
        .with_context(|| {
            format!(
                "Tried to run '{}' with arguments {:?}",
                command, command_args
            )
        })?;

    if output.status.success() {
        let std_out = std::str::from_utf8(&output.stdout)?;
        print!("{}", std_out);
        let std_err = std::str::from_utf8(&output.stderr)?;
        eprint!("{}", std_err);
    }

    std::process::exit(output.status.code().unwrap_or(0));
}

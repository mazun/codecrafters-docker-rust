use anyhow::{Context, Result};
use libc::{strerror, unshare, CLONE_NEWNS, CLONE_NEWPID};
use std::{env, ffi::CStr, fs, os::unix::fs::chroot};
use tempfile::TempDir;

mod docker_api;
use docker_api::DockerAPI;

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
#[tokio::main]
async fn main() -> Result<()> {
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    // println!("Logs from your program will appear here!");

    let args: Vec<_> = std::env::args().collect();
    if args.len() < 4 {
        return Ok(());
    }

    let name_reference = &args[2];
    // eprintln!("{}", name_reference);
    let name_reference: Vec<&str> = name_reference.split(":").collect();
    let name = name_reference[0];
    let reference = *name_reference.get(1).unwrap_or(&"latest");
    let command = &args[3];
    let command_args = &args[4..];

    let docker_api = DockerAPI::new();
    let tmp_dir = TempDir::new()?;
    docker_api.pull(name, reference, tmp_dir.path()).await?;

    check_libc_return_code(|| {
        let flags = CLONE_NEWPID | CLONE_NEWNS;
        unsafe { unshare(flags) }
    })?;

    chroot(tmp_dir.path())?;
    env::set_current_dir("/")?;

    fs::create_dir_all("/dev")?;
    fs::File::create("/dev/null")?;

    let output = std::process::Command::new(command)
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

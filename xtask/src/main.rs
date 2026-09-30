use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let task = args.next().unwrap_or_else(|| "help".to_string());

    if args.next().is_some() {
        return Err("unexpected extra arguments; run `cargo xtask help`".into());
    }

    match task.as_str() {
        "help" | "-h" | "--help" => print_help(),
        "build" => cargo_build(),
        "build-c" => build_c(false),
        "build-cpp" => build_c(true),
        "build-all" => {
            cargo_build()?;
            build_c(false)?;
            build_c(true)?;
            Ok(())
        }
        "run-c" => {
            build_c(false)?;
            run_binary("c_demo")
        }
        "run-cpp" => {
            build_c(true)?;
            run_binary("cpp_demo")
        }
        "clean-demo" => clean_demo_artifacts(),
        other => Err(format!(
            "unknown task `{other}`; run `cargo xtask help`"
        )),
    }
}

fn print_help() -> Result<(), String> {
    println!(
        r#"Available tasks:

  cargo xtask build       Build the Rust FFI dynamic library
  cargo xtask build-c     Build Rust + C demo
  cargo xtask build-cpp   Build Rust + C++ demo
  cargo xtask build-all   Build Rust + C + C++ demos
  cargo xtask run-c       Build and run the C demo
  cargo xtask run-cpp     Build and run the C++ demo
  cargo xtask clean-demo  Remove generated C/C++ demo executables

Environment overrides:
  CC   C compiler (Unix-like systems), default: gcc on Linux, clang on macOS
  CXX  C++ compiler (Unix-like systems), default: g++ on Linux, clang++ on macOS

Windows MSVC:
  Run from a Visual Studio Developer Command Prompt so `cl.exe` is available.
"#
    );
    Ok(())
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be placed directly under the workspace root")
        .to_path_buf()
}

fn target_release_dir() -> PathBuf {
    project_root().join("target").join("release")
}

fn demo_output_dir() -> PathBuf {
    project_root().join("target").join("ffi-demo")
}

fn cargo_build() -> Result<(), String> {
    run_command(
        Command::new(cargo_command())
            .current_dir(project_root())
            .args(["build", "--release", "-p", "flower-ffi"]),
        "cargo build --release -p flower-ffi",
    )
}

fn build_c(cpp: bool) -> Result<(), String> {
    cargo_build()?;

    let root = project_root();
    let out_dir = demo_output_dir();
    fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    if cfg!(target_os = "windows") {
        build_windows(&root, &out_dir, cpp)
    } else {
        build_unix(&root, &out_dir, cpp)
    }
}

fn build_unix(root: &Path, out_dir: &Path, cpp: bool) -> Result<(), String> {
    let (default_compiler, source, output) = if cpp {
        let compiler = if cfg!(target_os = "macos") { "clang++" } else { "g++" };
        (compiler, root.join("examples").join("main.cpp"), out_dir.join("cpp_demo"))
    } else {
        let compiler = if cfg!(target_os = "macos") { "clang" } else { "gcc" };
        (compiler, root.join("examples").join("main.c"), out_dir.join("c_demo"))
    };

    let compiler_var = if cpp { "CXX" } else { "CC" };
    let compiler = env::var_os(compiler_var)
        .unwrap_or_else(|| default_compiler.into());

    let include_dir = root.join("include");
    let library_dir = target_release_dir();

    let mut cmd = Command::new(&compiler);
    cmd.current_dir(root)
        .arg("-I")
        .arg(&include_dir)
        .arg(&source)
        .arg("-L")
        .arg(&library_dir)
        .arg("-lflower");

    if cfg!(target_os = "macos") {
        cmd.arg("-Wl,-rpath,@loader_path/../release");
    } else {
        cmd.arg("-Wl,-rpath,$ORIGIN/../release");
    }

    if cpp {
        cmd.arg("-std=c++17");
    } else {
        cmd.arg("-std=c11");
    }

    cmd.arg("-o").arg(&output);

    run_command(
        &mut cmd,
        &format!(
            "{} {} build",
            compiler.to_string_lossy(),
            source.display()
        ),
    )
}

fn build_windows(root: &Path, out_dir: &Path, cpp: bool) -> Result<(), String> {
    let source = if cpp {
        root.join("examples").join("main.cpp")
    } else {
        root.join("examples").join("main.c")
    };

    let output = if cpp {
        out_dir.join("cpp_demo.exe")
    } else {
        out_dir.join("c_demo.exe")
    };

    let import_lib = target_release_dir().join("flower_ffi.dll.lib");
    let dll = target_release_dir().join("flower_ffi.dll");

    let mut cmd = Command::new("cl");
    cmd.current_dir(root)
        .arg(if cpp { "/std:c++17" } else { "/std:c17" });

    if cpp {
        cmd.arg("/EHsc");
    }

    cmd.arg("/I")
        .arg(root.join("include"))
        .arg(&source)
        .arg("/link")
        .arg(format!("/LIBPATH:{}", target_release_dir().display()))
        .arg(&import_lib)
        .arg(format!("/OUT:{}", output.display()));

    run_command(
        &mut cmd,
        &format!("cl {}", source.display()),
    )?;

    fs::copy(&dll, out_dir.join("flower_ffi.dll"))
        .map_err(|e| format!("copy {}: {e}", dll.display()))?;

    Ok(())
}

fn run_binary(name: &str) -> Result<(), String> {
    let path = if cfg!(target_os = "windows") {
        demo_output_dir().join(format!("{name}.exe"))
    } else {
        demo_output_dir().join(name)
    };

    run_command(
        Command::new(&path).current_dir(project_root()),
        &format!("run {}", path.display()),
    )
}

fn clean_demo_artifacts() -> Result<(), String> {
    let dir = demo_output_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
    }
    Ok(())
}

fn run_command(command: &mut Command, description: &str) -> Result<(), String> {
    eprintln!("+ {description}");

    let status = command
        .status()
        .map_err(|e| format!("failed to start `{description}`: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`{description}` failed with {}",
            exit_status(status)
        ))
    }
}

fn exit_status(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "terminated by signal".into(),
    }
}

fn cargo_command() -> String {
    env::var_os("CARGO")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cargo".into())
}


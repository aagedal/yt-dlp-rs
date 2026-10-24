//! Compiled by the challenge supervisor tests as a native fake runtime.
use std::{
    io::{Read, Write},
    time::Duration,
};
/// The canned solver output for the `n` challenge `a`.
const SOLVED: &str = r#"{"type":"result","responses":[{"type":"result","data":{"a":"solved"}}]}"#;

/// The solver script for the `n` challenge `a`.
fn check_script(script: &str) {
    assert!(script.contains("Object.assign(globalThis, lib);"), "{script}");
    assert!(
        script.contains(r#""requests":[{"type":"n","challenges":["a"]}]"#),
        "{script}"
    );
}

/// A copy named `KIND@VERSION`: that runtime's version output, command
/// line and solver output.
fn named(kind: &str, version: &str, args: &[String]) {
    let stdin = || {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        input
    };
    match (kind, args.iter().map(String::as_str).collect::<Vec<_>>().as_slice()) {
        ("node", ["--version"]) => println!("v{version}"),
        ("bun", ["--version"]) => println!("{version}"),
        // `qjs --help` exits with status 1.
        ("qjs" | "qjs-ng", ["--help"]) => {
            let name = if kind == "qjs" { "QuickJS" } else { "QuickJS-ng" };
            println!("{name} version {version}\nusage: qjs [options] [file [args]]");
            std::process::exit(1);
        }
        ("bun", ["--bun", "run", "--no-addons", "--prefer-offline", "--no-install", "-"]) => {
            assert_eq!(
                std::env::var("BUN_RUNTIME_TRANSPILER_CACHE_PATH").as_deref(),
                Ok("0")
            );
            check_script(&stdin());
            if version.ends_with("-fail") {
                eprintln!("boom");
            } else {
                // Bun's banner is cleaned from stderr like upstream.
                eprintln!("Bun v{version} (macOS arm64)");
                print!("{SOLVED}");
            }
        }
        ("qjs" | "qjs-ng", ["--script", path]) => {
            assert!(path.ends_with(".js"), "{path}");
            assert!(std::path::Path::new(path).is_absolute(), "{path}");
            assert_eq!(stdin(), "");
            check_script(&std::fs::read_to_string(path).unwrap());
            println!("{SOLVED}");
        }
        ("node", ["--permission", "-"]) if !version.starts_with("22.") => {
            check_script(&stdin());
            println!("{SOLVED}");
        }
        ("node", ["--experimental-permission", "--no-warnings=ExperimentalWarning", "-"])
            if version.starts_with("22.") =>
        {
            check_script(&stdin());
            println!("{SOLVED}");
        }
        _ => panic!("unexpected {kind} {version} arguments {args:?}"),
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let program = std::env::current_exe().unwrap();
    let name = program.file_name().unwrap().to_string_lossy().into_owned();
    let stem = name.strip_suffix(std::env::consts::EXE_SUFFIX).unwrap_or(&name);
    if let Some((kind, version)) = stem.split_once('@') {
        return named(kind, version, &args);
    }
    if args == ["--version"] {
        println!("deno 2.9.7 (stable, release)\nv8 fake\ntypescript fake");
        return;
    }
    let mode = args.first().unwrap().as_str();
    // Isolate output-budget tests from stdin failure races: a reader hitting
    // its cap can close the pipe and make the helper exit before stdin drains.
    if matches!(mode, "stdout-flood" | "stderr-flood") {
        std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
    }
    match mode {
        "blocked-input" => std::thread::sleep(Duration::from_secs(30)),
        "drained-input-hang" => {
            std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "stdout-flood" => loop {
            if std::io::stdout().write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        },
        "stderr-flood" => loop {
            if std::io::stderr().write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        },
        "stderr" => {
            std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
            eprintln!("diagnostic");
        }
        "exit" => std::process::exit(7),
        "invalid-utf8" => {
            std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
            std::io::stdout().write_all(&[0xff]).unwrap();
        }
        "run" => {
            assert_eq!(
                args,
                [
                    "run",
                    "--ext=js",
                    "--no-code-cache",
                    "--no-prompt",
                    "--no-remote",
                    "--no-lock",
                    "--node-modules-dir=none",
                    "--no-config",
                    "--no-npm",
                    "--cached-only",
                    "-"
                ]
            );
            let cwd = std::env::current_dir().unwrap();
            assert_eq!(
                std::path::PathBuf::from(std::env::var_os("DENO_DIR").unwrap())
                    .canonicalize()
                    .unwrap(),
                cwd.canonicalize().unwrap()
            );
            for (key, _) in std::env::vars_os() {
                assert!(
                    [
                        "DENO_DIR",
                        "DENO_NO_UPDATE_CHECK",
                        "DENO_NO_PACKAGE_JSON",
                        "NO_COLOR",
                        "BUN_RUNTIME_TRANSPILER_CACHE_PATH",
                        "SystemRoot",
                        "WINDIR"
                    ]
                    .iter()
                    .any(|name| key == *name),
                    "unexpected environment {:?}",
                    key
                );
            }
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            assert_eq!(input, "check");
            print!("{}", cwd.display());
        }
        _ => panic!("unknown mode"),
    }
}

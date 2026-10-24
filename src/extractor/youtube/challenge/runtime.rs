//! Bounded process supervision. No shell, ambient Deno config, or script grants
//! (Deno without permissions, Node.js under its permission model); Bun and
//! QuickJS run upstream's command lines without npm installs.
use super::{Batch, JsRuntimeName, MAX_STDOUT, Result, Solutions};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{LazyLock, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Upstream `DenoJCP`'s `run` options for the builtin (non-npm) solver.
const ARGS: &[&str] = &[
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
    "-",
];
/// Upstream `BunJCP._run_js_runtime` for the builtin (non-npm) solver.
const BUN_ARGS: &[&str] = &[
    "--bun",
    "run",
    "--no-addons",
    "--prefer-offline",
    "--no-install",
    "-",
];
/// Upstream `DenoJsRuntime.MIN_SUPPORTED_VERSION`.
const MIN_DENO: &[i64] = &[2, 3, 0];
/// Upstream `NodeJsRuntime.MIN_SUPPORTED_VERSION`.
const MIN_NODE: &[i64] = &[22, 0, 0];
/// Upstream `BunJsRuntime.MIN_SUPPORTED_VERSION`.
const MIN_BUN: &[i64] = &[1, 2, 11];
/// Upstream `BunJCP._BUN_MAX_SUPPORTED_VERSION`.
const MAX_BUN: &[i64] = &[1, 3, 14];
/// Upstream `QuickJsRuntime.MIN_SUPPORTED_VERSION` (QuickJS-ng: any).
const MIN_QUICKJS: &[i64] = &[2023, 12, 9];
const MAX_STDERR: usize = 64 * 1024;
const DEADLINE: Duration = Duration::from_secs(15);

/// A supported runtime, as upstream's `JsRuntimeInfo` with its provider.
pub(in crate::extractor::youtube) struct DenoRuntime {
    executable: PathBuf,
    /// The provider (`deno`, `node`, `quickjs` or `bun`).
    name: &'static str,
    /// `JsRuntimeInfo.name` (`quickjs-ng` for QuickJS-ng).
    runtime: &'static str,
    version: Vec<i64>,
}

/// Upstream `NodeJCP._run_js_runtime`: the permission model (stable since
/// Node.js 23.5.0) and the script from stdin.
fn node_args(version: &[i64]) -> &'static [&'static str] {
    if version < &[23, 5, 0][..] {
        &[
            "--experimental-permission",
            "--no-warnings=ExperimentalWarning",
            "-",
        ]
    } else {
        &["--permission", "-"]
    }
}

/// Python `int(part)` for `version_tuple`'s parts.
fn py_int(part: &str) -> Option<i64> {
    let part = part.trim();
    let digits = part.strip_prefix(['+', '-']).unwrap_or(part);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// `version_tuple(text, lenient=True)`: `re.split(r'[-.]', v)`, each part
/// an integer or -1.
fn version_tuple(text: &str) -> Vec<i64> {
    text.split(['-', '.'])
        .map(|part| py_int(part).unwrap_or(-1))
        .collect()
}

/// `'.'.join(map(str, version_tuple))`.
fn dotted(version: &[i64]) -> String {
    version
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// `detect_exe_version(out, r'^PREFIX(\S+)', 'unknown')`: the
/// non-whitespace run after the prefix at the very start of the output.
fn detect(out: &str, prefix: &str) -> String {
    out.strip_prefix(prefix)
        .map(|rest| {
            rest.chars()
                .take_while(|c| !c.is_whitespace())
                .collect::<String>()
        })
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// `^QuickJS(?:-ng)?\s+version\s+(\S+)`.
fn detect_quickjs(out: &str) -> String {
    out.strip_prefix("QuickJS")
        .map(|rest| rest.strip_prefix("-ng").unwrap_or(rest))
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .and_then(|rest| rest.trim_start().strip_prefix("version"))
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .map(|rest| detect(rest.trim_start(), ""))
        .unwrap_or_else(|| "unknown".into())
}

type Selected = (&'static str, &'static str, Vec<i64>);

/// The provider, runtime name and version tuple for a runtime's version
/// output (`JsRuntime._info`), or why it is not supported.
fn select(name: JsRuntimeName, out: &str) -> Result<Selected> {
    fn at_least(selected: Selected, minimum: &[i64]) -> Result<Selected> {
        if selected.2.as_slice() >= minimum {
            Ok(selected)
        } else {
            Err(format!(
                "unsupported {} version {}",
                selected.1,
                dotted(&selected.2)
            ))
        }
    }
    match name {
        JsRuntimeName::Deno => at_least(
            ("deno", "deno", version_tuple(&detect(out, "deno "))),
            MIN_DENO,
        ),
        JsRuntimeName::Node => {
            at_least(("node", "node", version_tuple(&detect(out, "v"))), MIN_NODE)
        }
        JsRuntimeName::Bun => at_least(("bun", "bun", version_tuple(&detect(out, ""))), MIN_BUN),
        JsRuntimeName::QuickJs => {
            let version = version_tuple(&detect_quickjs(out));
            if out.contains("QuickJS-ng") {
                // `supported=vt > (0,)`.
                if version.as_slice() > &[0][..] {
                    Ok(("quickjs", "quickjs-ng", version))
                } else {
                    Err(format!(
                        "unsupported quickjs-ng version {}",
                        dotted(&version)
                    ))
                }
            } else {
                at_least(("quickjs", "quickjs", version), MIN_QUICKJS)
            }
        }
        // `--youtube-js-runtime`: Deno or Node.js by its `--version` line.
        JsRuntimeName::Detect if out.starts_with("deno ") => select(JsRuntimeName::Deno, out),
        JsRuntimeName::Detect if out.starts_with('v') => select(JsRuntimeName::Node, out),
        JsRuntimeName::Detect => {
            Err("YouTube solving requires Deno 2.3.0+ or Node.js 22.0.0+".into())
        }
    }
}

/// Upstream `DenoJCP._clean_stderr` (after `remove_terminal_sequences`).
fn clean_deno(stderr: &str) -> String {
    static SEQUENCES: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new("\x1b\\[[^m]+m").unwrap());
    static DOWNLOAD: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^Download\s+https\S+$").unwrap());
    stderr
        .lines()
        .filter(|line| {
            let line = SEQUENCES.replace_all(line, "");
            !DOWNLOAD.is_match(&line)
                && !line
                    .starts_with("DANGER: TLS certificate validation is disabled for all hostnames")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Upstream `NodeJCP._clean_stderr`.
fn clean_node(stderr: &str) -> String {
    static VERSION: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^Node\.js v\d+\.\d+\.\d+$").unwrap());
    stderr
        .lines()
        .filter(|line| {
            !(line.starts_with("[stdin]:")
                || line.starts_with("var jsc")
                || *line
                    == "(Use `node --trace-uncaught ...` to show where the exception was thrown)"
                || VERSION.is_match(line))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Upstream `BunJCP._clean_stderr`.
fn clean_bun(stderr: &str) -> String {
    static VERSION: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^Bun v\d+\.\d+\.\d+ \([\w\s]+\)$").unwrap());
    stderr
        .lines()
        .filter(|line| !VERSION.is_match(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Upstream `QuickJSJCP` keeps all of stderr.
fn clean_none(stderr: &str) -> String {
    stderr.to_owned()
}

impl DenoRuntime {
    /// Explicit path only: discovery and installation are the caller's
    /// concern. The version comes from `--version` (QuickJS: `--help`,
    /// whose exit status is ignored), with stderr after stdout.
    pub(in crate::extractor::youtube) fn new(
        name: JsRuntimeName,
        executable: &Path,
    ) -> Result<Self> {
        let executable = executable
            .canonicalize()
            .map_err(|e| format!("JavaScript runtime executable: {e}"))?;
        let (args, ignore_status): (&[&str], bool) = match name {
            JsRuntimeName::QuickJs => (&["--help"], true),
            _ => (&["--version"], false),
        };
        let output = execute(
            &executable,
            args,
            Input::Stdin(Vec::new()),
            DEADLINE,
            4096,
            Mode::Version { ignore_status },
        )?;
        let out = std::str::from_utf8(&output).map_err(|_| "invalid runtime version output")?;
        // `_get_exe_version_output` without output: not available.
        if out.is_empty() {
            return Err("JavaScript runtime printed no version".into());
        }
        let (name, runtime, version) = select(name, out)?;
        Ok(Self {
            executable,
            name,
            runtime,
            version,
        })
    }

    /// The JS challenge provider's name (`deno`, `node`, `quickjs` or `bun`).
    pub(in crate::extractor::youtube) fn name(&self) -> &'static str {
        self.name
    }

    /// The provider's `_run_js_runtime` messages, command line and stderr
    /// cleaning.
    fn invocation(&self, script: Vec<u8>) -> (&'static [&'static str], Input, Mode) {
        let prefix = format!("[youtube] [jsc:{}]", self.name);
        match self.name {
            "deno" => (ARGS, Input::Stdin(script), Mode::Solve(clean_deno)),
            "node" => (
                node_args(&self.version),
                Input::Stdin(script),
                Mode::Solve(clean_node),
            ),
            "bun" => {
                if self.version.as_slice() > MAX_BUN {
                    crate::screen::warning_once(&format!(
                        "{prefix} bun version {} is not supported! {} is the last supported bun version. DO NOT open a bug report even if you encounter any errors!",
                        dotted(&self.version),
                        dotted(MAX_BUN)
                    ));
                } else {
                    crate::screen::screen_once(&format!(
                        "{prefix} bun support has been deprecated. See  https://github.com/yt-dlp/yt-dlp/issues/16766  for details"
                    ));
                }
                (BUN_ARGS, Input::Stdin(script), Mode::Solve(clean_bun))
            }
            _ => {
                // `_QJS_MIN_RECOMMENDED`.
                let recommended: &[i64] = if self.runtime == "quickjs-ng" {
                    &[0, 12, 0]
                } else {
                    &[2025, 4, 26]
                };
                if self.version.as_slice() < recommended {
                    crate::screen::warning(&format!(
                        "{prefix} {} versions older than {} are missing important optimizations and will solve the JS challenges very slowly. Consider upgrading.",
                        self.runtime,
                        dotted(recommended)
                    ));
                }
                // QuickJS does not read the script from stdin.
                (
                    &["--script"],
                    Input::ScriptFile(script),
                    Mode::Solve(clean_none),
                )
            }
        }
    }

    pub(in crate::extractor::youtube) fn solve(&self, batch: &Batch) -> Result<Solutions> {
        let script = batch.script()?.into_bytes();
        let (args, input, mode) = self.invocation(script);
        let output = execute(&self.executable, args, input, DEADLINE, MAX_STDOUT, mode)?;
        batch.validate(&output)
    }
}

/// The solver script: on stdin, or (QuickJS) in a private `.js` file
/// whose path follows the arguments.
enum Input {
    Stdin(Vec<u8>),
    ScriptFile(Vec<u8>),
}

#[derive(Clone, Copy)]
enum Mode {
    /// A solver: a failure status, or any diagnostic the provider's
    /// cleaner keeps, is an error.
    Solve(fn(&str) -> String),
    /// `_get_exe_version_output`: stderr follows stdout; a failure status
    /// is an error unless ignored.
    Version { ignore_status: bool },
}

enum Event {
    Input(Result<()>),
    Output(Result<Vec<u8>>),
    Error(Result<Vec<u8>>),
}

fn read_pipe(mut pipe: impl Read, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = pipe
            .read(&mut buffer)
            .map_err(|e| format!("solver pipe read: {e}"))?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > cap {
            return Err("solver output budget exceeded".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn execute(
    executable: &Path,
    args: &[&str],
    input: Input,
    deadline: Duration,
    stdout_cap: usize,
    mode: Mode,
) -> Result<Vec<u8>> {
    let start = Instant::now();
    let directory = tempfile::tempdir().map_err(|e| format!("solver private directory: {e}"))?;
    let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let input = match input {
        Input::Stdin(bytes) => bytes,
        Input::ScriptFile(script) => {
            let script_error = |e: std::io::Error| format!("solver script file: {e}");
            let mut file = tempfile::Builder::new()
                .suffix(".js")
                .tempfile_in(directory.path())
                .map_err(script_error)?;
            file.write_all(&script).map_err(script_error)?;
            // Removed with the private directory.
            let (_, path) = file.keep().map_err(|e| script_error(e.error))?;
            args.push(path.into_os_string());
            Vec::new()
        }
    };
    let mut command = Command::new(executable);
    command
        .args(&args)
        .env_clear()
        .current_dir(directory.path())
        .env("DENO_DIR", directory.path())
        .env("DENO_NO_UPDATE_CHECK", "1")
        .env("DENO_NO_PACKAGE_JSON", "1")
        // Upstream `BunJCP._get_env_options`: no transpiler cache.
        .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "0")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows needs these for OS/DLL lookup; none are runtime flags or caches.
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start solver runtime: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (sender, receiver) = mpsc::channel();
    // Scoped threads ensure pipes and private state cannot outlive this call.
    let result = thread::scope(|scope| {
        let tx = sender.clone();
        let spawned = thread::Builder::new().spawn_scoped(scope, move || {
            let result = stdin
                .write_all(&input)
                .map_err(|e| format!("solver stdin write: {e}"));
            drop(stdin);
            let _ = tx.send(Event::Input(result));
        });
        if let Err(error) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("solver helper thread: {error}"));
        }
        let tx = sender.clone();
        let spawned = thread::Builder::new().spawn_scoped(scope, move || {
            let _ = tx.send(Event::Output(read_pipe(stdout, stdout_cap)));
        });
        if let Err(error) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("solver helper thread: {error}"));
        }
        let spawned = thread::Builder::new().spawn_scoped(scope, move || {
            let _ = sender.send(Event::Error(read_pipe(stderr, MAX_STDERR)));
        });
        if let Err(error) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("solver helper thread: {error}"));
        }
        let mut input_done = false;
        let mut output = None;
        let mut errors = None;
        let result = loop {
            if start.elapsed() >= deadline {
                break Err("solver wall deadline exceeded".into());
            }
            match receiver.recv_timeout(
                Duration::from_millis(5).min(deadline.saturating_sub(start.elapsed())),
            ) {
                Ok(Event::Input(Ok(()))) => input_done = true,
                Ok(Event::Output(Ok(bytes))) => output = Some(bytes),
                Ok(Event::Error(Ok(bytes))) => {
                    if let Mode::Solve(clean) = mode {
                        let kept = clean(&String::from_utf8_lossy(&bytes));
                        if !kept.is_empty() {
                            break Err(super::solver_error(&kept));
                        }
                    }
                    errors = Some(bytes);
                }
                Ok(
                    Event::Input(Err(error)) | Event::Output(Err(error)) | Event::Error(Err(error)),
                ) => break Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    thread::sleep(Duration::from_millis(5))
                }
            }
            match child.try_wait() {
                Ok(Some(status))
                    if !status.success()
                        && !matches!(
                            mode,
                            Mode::Version {
                                ignore_status: true
                            }
                        ) =>
                {
                    break Err(format!("solver exited with {status}"));
                }
                Ok(Some(_)) if input_done && output.is_some() && errors.is_some() => {
                    let mut bytes = output.take().unwrap();
                    if let Mode::Version { .. } = mode {
                        bytes.extend(errors.take().unwrap());
                    }
                    break Ok(bytes);
                }
                Ok(_) => {}
                Err(error) => break Err(format!("solver process status: {error}")),
            }
        };
        if result.is_err() {
            // Killing closes child pipe ends, unblocking readers and the writer.
            // The Deno invocation grants no subprocess permission.
            let _ = child.kill();
        }
        let waited = child
            .wait()
            .map_err(|e| format!("solver process wait: {e}"));
        match (result, waited) {
            (Err(error), _) => Err(error),
            (Ok(bytes), Ok(_)) => Ok(bytes),
            (Ok(_), Err(error)) => Err(error),
        }
    });
    match (result, directory.close()) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(format!("solver private directory cleanup: {error}")),
        (Err(error), Err(cleanup)) => Err(format!(
            "{error}; solver private directory cleanup: {cleanup}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    // Compile a portable native helper, never a shell script or Python runtime.
    fn fake() -> &'static Path {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        HELPER.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/challenge_runtime.rs");
            let binary = directory.join(format!("fake-runtime{}", std::env::consts::EXE_SUFFIX));
            let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(source)
                .arg("--edition=2024")
                .arg("-Dwarnings")
                .arg("-o")
                .arg(&binary)
                .status()
                .unwrap();
            assert!(status.success());
            binary
        })
    }

    #[test]
    fn arguments_environment_and_private_directory() {
        let output = execute(
            fake(),
            ARGS,
            Input::Stdin(b"check".to_vec()),
            Duration::from_secs(5),
            MAX_STDOUT,
            Mode::Solve(clean_deno),
        )
        .unwrap();
        let directory = PathBuf::from(String::from_utf8(output).unwrap());
        assert!(
            !directory.exists(),
            "private runtime directory must be removed"
        );
        assert!(DenoRuntime::new(JsRuntimeName::Deno, fake()).is_ok());
        assert!(DenoRuntime::new(JsRuntimeName::Detect, fake()).is_ok());
    }

    #[test]
    fn bounded_failures_reap_child_and_join_pipes() {
        let executable = fake();
        for mode in [
            "blocked-input",
            "stdout-flood",
            "stderr-flood",
            "stderr",
            "exit",
            "drained-input-hang",
            "invalid-utf8",
        ] {
            let before = Instant::now();
            let args = [mode];
            let result = execute(
                executable,
                &args,
                Input::Stdin(vec![b'x'; 2 * 1024 * 1024]),
                if mode.ends_with("hang") || mode == "blocked-input" {
                    Duration::from_millis(150)
                } else {
                    Duration::from_secs(2)
                },
                1024,
                Mode::Solve(clean_none),
            );
            if mode == "invalid-utf8" {
                // The process boundary preserves bytes; protocol validation rejects them.
                assert_eq!(result.unwrap(), vec![0xff]);
            } else {
                let error = result.unwrap_err();
                let expected = match mode {
                    "blocked-input" | "drained-input-hang" => "deadline",
                    "stdout-flood" | "stderr-flood" => "output budget",
                    "stderr" => "diagnostic",
                    "exit" => "solver", // status or a raced broken stdin pipe
                    _ => unreachable!(),
                };
                assert!(error.contains(expected), "{mode}: {error}");
            }
            assert!(before.elapsed() < Duration::from_secs(5), "{mode}");
        }
    }

    #[test]
    fn real_runtime_denies_script_permissions() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_DENO") else {
            return;
        };
        let runtime = DenoRuntime::new(JsRuntimeName::Detect, Path::new(&path)).unwrap();
        let script = br#"
const actions = [
    () => Deno.env.get('DENO_DIR'),
    () => Deno.readTextFile('private-file'),
    () => Deno.writeTextFile('private-file', 'data'),
    () => fetch('http://127.0.0.1:18080/'),
    () => new Deno.Command(Deno.execPath(), {args: ['--version']}).output(),
];
for (const action of actions) {
    try { await action(); throw new Error('permission unexpectedly granted'); }
    catch (error) { if (error.name !== 'NotCapable') throw error; }
}
console.log('denied');
"#;
        let (args, input, mode) = runtime.invocation(script.to_vec());
        let output = execute(&runtime.executable, args, input, DEADLINE, 4096, mode).unwrap();
        assert_eq!(String::from_utf8(output).unwrap().trim(), "denied");
    }

    #[test]
    fn deno_versions_from_upstream_minimum_are_supported() {
        for line in [
            "deno 2.3.0 (stable, release, x86_64-unknown-linux-gnu)",
            "deno 2.9.7 (stable, release, aarch64-apple-darwin)",
            "deno 3.0.0-rc.1 (canary)",
        ] {
            for name in [JsRuntimeName::Deno, JsRuntimeName::Detect] {
                assert_eq!(select(name, line).unwrap().0, "deno", "{line}");
            }
        }
        assert_eq!(
            select(JsRuntimeName::Deno, "deno 3.0.0-rc.1").unwrap().2,
            [3, 0, 0, -1, 1]
        );
        for line in ["deno 2.2.12 (stable, release)", "deno x", "bun 1.2.0", ""] {
            assert!(select(JsRuntimeName::Deno, line).is_err(), "{line}");
        }
        assert!(select(JsRuntimeName::Detect, "1.2.11").is_err());
    }

    #[test]
    fn node_versions_select_upstream_permission_flags() {
        assert_eq!(version_tuple("26.12.0"), [26, 12, 0]);
        assert_eq!(version_tuple("22.1"), [22, 1]);
        let (name, _, version) = select(JsRuntimeName::Node, "v26.12.0\n").unwrap();
        assert_eq!(name, "node");
        assert_eq!(node_args(&version), ["--permission", "-"]);
        assert!(select(JsRuntimeName::Node, "v21.9.0").is_err());
        assert!(select(JsRuntimeName::Node, "26.0.0").is_err());
        assert_eq!(node_args(&[23, 4, 9])[0], "--experimental-permission");
        assert_eq!(node_args(&[23, 5, 0]), ["--permission", "-"]);
    }

    #[test]
    fn bun_and_quickjs_versions_follow_upstream_runtime_info() {
        // `BunJsRuntime`: `^(\S+)`, at least 1.2.11.
        assert_eq!(
            select(JsRuntimeName::Bun, "1.2.11\n").unwrap(),
            ("bun", "bun", vec![1, 2, 11])
        );
        assert!(select(JsRuntimeName::Bun, "1.2.10").is_err());
        assert!(select(JsRuntimeName::Bun, "").is_err());
        // `QuickJsRuntime`: `--help`'s first line; dates split on `-`.
        let help = "QuickJS version 2025-04-26\nusage: qjs [options] [file [args]]\n";
        assert_eq!(
            select(JsRuntimeName::QuickJs, help).unwrap(),
            ("quickjs", "quickjs", vec![2025, 4, 26])
        );
        assert!(select(JsRuntimeName::QuickJs, "QuickJS version 2023-12-08\n").is_err());
        assert!(select(JsRuntimeName::QuickJs, "QuickJS version 2023-12-09").is_ok());
        assert_eq!(
            select(JsRuntimeName::QuickJs, "QuickJS-ng version 0.10.1\n").unwrap(),
            ("quickjs", "quickjs-ng", vec![0, 10, 1])
        );
        // `(0, 0, 0) > (0,)`; an unrecognized version is `(-1,)`.
        assert!(select(JsRuntimeName::QuickJs, "QuickJS-ng version 0.0.0").is_ok());
        assert!(select(JsRuntimeName::QuickJs, "QuickJS-ng version x").is_err());
        assert!(
            select(
                JsRuntimeName::QuickJs,
                "usage: qjs\nQuickJS version 2025-04-26"
            )
            .is_err()
        );
        assert_eq!(dotted(&version_tuple("1.4.0-canary.1")), "1.4.0.-1.1");
    }

    #[test]
    fn provider_stderr_cleaning() {
        assert_eq!(clean_bun("Bun v1.2.11 (macOS arm64)\n"), "");
        assert_eq!(clean_bun("Bun v1.2.11 (macOS arm64)\nboom"), "boom");
        assert_eq!(
            clean_node("[stdin]:1\nvar jsc = 1\nNode.js v22.0.0\nError: x"),
            "Error: x"
        );
        assert_eq!(
            clean_deno("\x1b[0m\x1b[32mDownload\x1b[0m https://registry.npmjs.org/x\n"),
            ""
        );
        assert_eq!(clean_none("x\n"), "x\n");
    }

    /// A named copy of the fake: `KIND@VERSION` selects its behavior.
    /// Copies share the helper's directory; each test uses its own names.
    fn named_fake(name: &str) -> PathBuf {
        let path = fake().with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if !path.exists() {
            std::fs::copy(fake(), &path).unwrap();
        }
        path
    }

    fn solve_one(runtime: &DenoRuntime) -> Result<Option<String>> {
        let batch = Batch::new("player", &[(super::super::Kind::N, "a")])?;
        let solved = runtime.solve(&batch)?;
        Ok(solved.get(super::super::Kind::N, "a").map(str::to_owned))
    }

    #[test]
    fn bun_runs_upstream_command_line_from_stdin() {
        let runtime = DenoRuntime::new(JsRuntimeName::Bun, &named_fake("bun@1.2.11")).unwrap();
        assert_eq!((runtime.name(), runtime.runtime), ("bun", "bun"));
        // The fake checks the arguments, environment and stdin script, and
        // writes Bun's crash banner (cleaned like upstream) to stderr.
        assert_eq!(solve_one(&runtime).unwrap().as_deref(), Some("solved"));
        assert!(DenoRuntime::new(JsRuntimeName::Bun, &named_fake("bun@1.2.10")).is_err());
        let failing = DenoRuntime::new(JsRuntimeName::Bun, &named_fake("bun@1.3.15-fail")).unwrap();
        assert_eq!(failing.version, [1, 3, 15, -1]);
        assert!(solve_one(&failing).unwrap_err().contains("boom"));
    }

    #[test]
    fn quickjs_runs_a_script_file_and_detects_ng() {
        let runtime =
            DenoRuntime::new(JsRuntimeName::QuickJs, &named_fake("qjs@2025-04-26")).unwrap();
        assert_eq!((runtime.name(), runtime.runtime), ("quickjs", "quickjs"));
        assert_eq!(runtime.version, [2025, 4, 26]);
        assert_eq!(solve_one(&runtime).unwrap().as_deref(), Some("solved"));
        let ng = DenoRuntime::new(JsRuntimeName::QuickJs, &named_fake("qjs-ng@0.10.1")).unwrap();
        assert_eq!((ng.name(), ng.runtime), ("quickjs", "quickjs-ng"));
        assert_eq!(solve_one(&ng).unwrap().as_deref(), Some("solved"));
        assert!(DenoRuntime::new(JsRuntimeName::QuickJs, &named_fake("qjs@2023-01-01")).is_err());
    }

    #[test]
    fn node_runs_under_the_permission_model() {
        let runtime = DenoRuntime::new(JsRuntimeName::Node, &named_fake("node@23.5.0")).unwrap();
        assert_eq!(solve_one(&runtime).unwrap().as_deref(), Some("solved"));
        let old = DenoRuntime::new(JsRuntimeName::Node, &named_fake("node@22.1.0")).unwrap();
        assert_eq!(solve_one(&old).unwrap().as_deref(), Some("solved"));
        assert!(DenoRuntime::new(JsRuntimeName::Node, &named_fake("node@21.0.0")).is_err());
        // Each provider reads only its own version format.
        assert!(DenoRuntime::new(JsRuntimeName::Deno, &named_fake("node@23.5.0")).is_err());
    }

    #[test]
    fn real_node_denies_script_permissions() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_NODE") else {
            return;
        };
        let runtime = DenoRuntime::new(JsRuntimeName::Detect, Path::new(&path)).unwrap();
        let script = br#"
const actions = [
    () => require('fs').readFileSync('private-file'),
    () => require('fs').writeFileSync('private-file', 'data'),
    () => require('child_process').execFileSync(process.execPath, ['--version']),
];
for (const action of actions) {
    try { action(); throw new Error('permission unexpectedly granted'); }
    catch (error) { if (error.code !== 'ERR_ACCESS_DENIED') throw error; }
}
console.log('denied');
"#;
        let (args, input, mode) = runtime.invocation(script.to_vec());
        let output = execute(&runtime.executable, args, input, DEADLINE, 4096, mode).unwrap();
        assert_eq!(String::from_utf8(output).unwrap().trim(), "denied");
    }

    #[test]
    fn real_node_saved_player_vectors() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_NODE") else {
            return;
        };
        saved_player_vectors(&DenoRuntime::new(JsRuntimeName::Detect, Path::new(&path)).unwrap());
    }

    #[test]
    fn real_saved_player_vectors() {
        let Some(path) = std::env::var_os("YT_DLP_RS_TEST_DENO") else {
            return;
        };
        // Dedicated CI always sets this variable; a configured runtime must pass.
        saved_player_vectors(&DenoRuntime::new(JsRuntimeName::Detect, Path::new(&path)).unwrap());
    }

    fn saved_player_vectors(runtime: &DenoRuntime) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/youtube/challenges/expected.json"
        ))
        .unwrap();
        let player =
            include_str!("../../../../tests/fixtures/youtube/challenges/player-74edf1a3.js");
        let mut inputs = Vec::new();
        for request in fixture["requests"].as_array().unwrap() {
            let kind = match request["type"].as_str().unwrap() {
                "n" => super::super::Kind::N,
                "sig" => super::super::Kind::Sig,
                _ => panic!("fixture kind"),
            };
            for challenge in request["challenges"].as_object().unwrap().keys() {
                inputs.push((kind, challenge.as_str()));
            }
        }
        let batch = Batch::new(player, &inputs).unwrap();
        let solved = runtime.solve(&batch).unwrap();
        for request in fixture["requests"].as_array().unwrap() {
            let kind = if request["type"] == "n" {
                super::super::Kind::N
            } else {
                super::super::Kind::Sig
            };
            for (challenge, expected) in request["challenges"].as_object().unwrap() {
                assert_eq!(solved.get(kind, challenge), expected.as_str());
            }
        }
    }
}

//! Socket receive error metadata is checked on Interpreter, JS, and Native.
//! WASM profiles do not expose this OS/network surface.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "e38_socket_recv_error_info_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).expect("mkdir fixture");
    dir
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cc_available() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_command_stdout(mut cmd: Command, label: &str) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("{label}: {e}"));
    assert!(
        out.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_three_backends(main_path: &Path, dir: &Path) -> [(String, String); 3] {
    assert!(
        node_available(),
        "node is required for 3-backend E38 socket recv parity"
    );
    assert!(
        cc_available(),
        "cc is required for 3-backend E38 socket recv parity"
    );

    let mut interp_cmd = Command::new(common::taida_bin());
    interp_cmd.arg(main_path);
    let interp = run_command_stdout(interp_cmd, "interp run");

    let mjs = dir.join("main.mjs");
    let mut build_cmd = Command::new(common::taida_bin());
    build_cmd
        .args(["build", "js"])
        .arg(main_path)
        .arg("-o")
        .arg(&mjs);
    run_command_stdout(build_cmd, "js build");
    let mut run_cmd = Command::new("node");
    run_cmd.arg(&mjs);
    let js = run_command_stdout(run_cmd, "js run");

    let bin = dir.join("main.bin");
    let mut build_cmd = Command::new(common::taida_bin());
    build_cmd
        .args(["build", "native"])
        .arg(main_path)
        .arg("-o")
        .arg(&bin);
    run_command_stdout(build_cmd, "native build");
    let run_cmd = Command::new(&bin);
    let native = run_command_stdout(run_cmd, "native run");

    [
        ("interp".to_string(), interp),
        ("js".to_string(), js),
        ("native".to_string(), native),
    ]
}

#[test]
fn recv_lax_failures_carry_error_info_across_backends() {
    let dir = fixture_dir("invalid_handle");
    let main = dir.join("main.td");
    fs::write(
        &main,
        concat!(
            ">>> taida-lang/os => @(socketRecv, socketRecvBytes, socketRecvExact, udpRecvFrom)\n",
            "socketRecv(999999, 1) >=> badText\n",
            "textInfo <= badText.errorInfo()\n",
            "textInfo >=> textErr\n",
            "stdout(badText.hasValue().toString())\n",
            "stdout(textInfo.hasValue().toString())\n",
            "stdout(textErr.message)\n",
            "stdout(textErr.kind)\n",
            "socketRecvBytes(999999, 1) >=> badBytes\n",
            "bytesInfo <= badBytes.errorInfo()\n",
            "bytesInfo >=> bytesErr\n",
            "stdout(badBytes.hasValue().toString())\n",
            "stdout(bytesInfo.hasValue().toString())\n",
            "stdout(bytesErr.message)\n",
            "stdout(bytesErr.kind)\n",
            "socketRecvExact(999999, 4, 1) >=> badExact\n",
            "exactInfo <= badExact.errorInfo()\n",
            "exactInfo >=> exactErr\n",
            "stdout(badExact.hasValue().toString())\n",
            "stdout(exactInfo.hasValue().toString())\n",
            "stdout(exactErr.message)\n",
            "stdout(exactErr.kind)\n",
            "udpRecvFrom(999999, 1) >=> badUdp\n",
            "udpInfo <= badUdp.errorInfo()\n",
            "udpInfo >=> udpErr\n",
            "stdout(badUdp.hasValue().toString())\n",
            "stdout(udpInfo.hasValue().toString())\n",
            "stdout(udpErr.message)\n",
            "stdout(udpErr.kind)\n",
        ),
    )
    .expect("write main");

    let expected = concat!(
        "false\ntrue\nSocketRecv error\ninvalid\n",
        "false\ntrue\nSocketRecvBytes error\ninvalid\n",
        "false\ntrue\nSocketRecvExact error\ninvalid\n",
        "false\ntrue\nUdpRecvFrom error\ninvalid"
    );
    let results = run_three_backends(&main, &dir);
    for (backend, out) in results {
        assert_eq!(out, expected, "{} backend output mismatch", backend);
    }
    let _ = fs::remove_dir_all(&dir);
}

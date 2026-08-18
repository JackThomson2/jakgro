use std::io::Write;
use std::process::{Command, Stdio};

use jakgro::uci::run;

fn transcript(input: &str) -> String {
    let mut output = Vec::new();
    run(std::io::Cursor::new(input.as_bytes()), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn public_runner_handles_a_protocol_transcript() {
    assert_eq!(
        transcript("uci\nisready\nposition startpos\ngo searchmoves e2e4 depth 1\nquit\n"),
        concat!(
            "id name Jakgro ",
            env!("CARGO_PKG_VERSION"),
            "\nid author Jakgro contributors\nuciok\nreadyok\nbestmove e2e4\n"
        )
    );
}

#[test]
fn executable_is_wired_to_standard_io() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jakgro"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"uci\nisready\nposition startpos\ngo searchmoves e2e4 depth 1\nquit\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!(
            "id name Jakgro ",
            env!("CARGO_PKG_VERSION"),
            "\nid author Jakgro contributors\nuciok\nreadyok\nbestmove e2e4\n"
        )
    );
}

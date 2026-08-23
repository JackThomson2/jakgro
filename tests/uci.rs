use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use jakgro::uci::run;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn transcript(input: &str) -> String {
    let mut output = Vec::new();
    run(std::io::Cursor::new(input.as_bytes().to_vec()), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}
struct EngineProcess {
    child: Child,
    input: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl EngineProcess {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_jakgro"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (line_sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if line_sender.send(line).is_err() {
                    break;
                }
            }
        });

        Self {
            child,
            input: Some(input),
            lines,
        }
    }

    fn send(&mut self, command: &str) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{command}").unwrap();
        input.flush().unwrap();
    }

    fn receive_until<F>(&self, timeout: Duration, mut predicate: F) -> Vec<String>
    where
        F: FnMut(&str) -> bool,
    {
        let deadline = Instant::now() + timeout;
        let mut received = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) => {
                    let matched = predicate(&line);
                    received.push(line);
                    if matched {
                        return received;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    panic!("timed out waiting for engine output; received {received:?}");
                }
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("engine output closed; received {received:?}");
                }
            }
        }
    }

    fn assert_no_bestmove(&self, duration: Duration) {
        let deadline = Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) => assert!(!line.starts_with("bestmove "), "unexpected {line}"),
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
    fn close_input(&mut self) {
        self.input = None;
    }

    fn receive_to_end(&self, timeout: Duration) -> Vec<String> {
        let deadline = Instant::now() + timeout;
        let mut received = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) => received.push(line),
                Err(RecvTimeoutError::Disconnected) => return received,
                Err(RecvTimeoutError::Timeout) => {
                    panic!("timed out waiting for engine shutdown; received {received:?}");
                }
            }
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "engine did not exit in time");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        if let Some(input) = self.input.as_mut() {
            let _ = writeln!(input, "quit");
            let _ = input.flush();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn public_runner_handles_a_protocol_transcript() {
    assert_eq!(
        transcript("uci\nisready\nquit\n"),
        concat!(
            "id name Jakgro ",
            env!("CARGO_PKG_VERSION"),
            "\nid author Jakgro contributors\noption name Hash type spin default 16 min 1 max 1024\noption name Aggression type spin default 75 min 0 max 100\noption name Move Overhead type spin default 10 min 0 max 5000\noption name Clear Hash type button\nuciok\nreadyok\n"
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
        .write_all(b"uci\nisready\nquit\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(output.status.success());
    assert!(stdout.contains("uciok\nreadyok\n"));
}
#[test]
fn stop_interrupts_infinite_search_and_isready_stays_responsive() {
    let mut engine = EngineProcess::spawn();
    engine.send("position startpos");
    engine.send("go infinite");

    let progress = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("info depth "));
    assert!(progress.last().unwrap().contains(" score cp "));
    assert!(progress.last().unwrap().contains(" nodes "));
    assert!(progress.last().unwrap().contains(" time "));
    assert!(progress.last().unwrap().contains(" nps "));
    assert!(progress.last().unwrap().contains(" pv "));
    engine.assert_no_bestmove(Duration::from_millis(30));

    engine.send("setoption name Move Overhead value 250");
    engine.send("setoption name Aggression value 0");
    engine.send("isready");
    let ready = engine.receive_until(TEST_TIMEOUT, |line| line == "readyok");
    assert!(ready.iter().all(|line| !line.starts_with("bestmove ")));

    engine.send("stop");
    let stopped = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));
    assert_eq!(
        stopped
            .iter()
            .filter(|line| line.starts_with("bestmove "))
            .count(),
        1
    );

    engine.send("isready");
    let ready_again = engine.receive_until(TEST_TIMEOUT, |line| line == "readyok");
    assert!(
        ready_again
            .iter()
            .all(|line| !line.starts_with("bestmove "))
    );
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}

#[test]
fn ponder_result_is_withheld_until_ponderhit() {
    let mut engine = EngineProcess::spawn();
    engine.send("position startpos");
    engine.send("go ponder searchmoves e2e4 movetime 50");

    engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("info depth "));
    engine.assert_no_bestmove(Duration::from_millis(30));
    engine.send("ponderhit");
    let result = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    assert!(result.last().unwrap().starts_with("bestmove e2e4"));
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}

#[test]
fn replacement_search_suppresses_the_stale_result() {
    let mut engine = EngineProcess::spawn();
    engine.send("position startpos");
    engine.send("go infinite searchmoves e2e4");
    engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("info depth "));

    engine.send("go searchmoves d2d4 depth 1");
    let result = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    assert_eq!(result.last().unwrap(), "bestmove d2d4");
    assert!(result.iter().all(|line| line != "bestmove e2e4"));
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}

#[test]
fn movetime_search_finishes_without_another_command() {
    let mut engine = EngineProcess::spawn();
    engine.send("position startpos");
    engine.send("go searchmoves e2e4 movetime 20");

    let result = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    assert!(result.last().unwrap().starts_with("bestmove e2e4"));
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}
#[test]
fn clock_search_honors_persistent_move_overhead_and_hard_limit() {
    let mut engine = EngineProcess::spawn();
    engine.send("setoption name Move Overhead value 50");
    engine.send("ucinewgame");
    engine.send("position startpos");
    let started = Instant::now();
    engine.send("go searchmoves e2e4 wtime 500 btime 500 movestogo 30");

    let result = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    assert!(result.last().unwrap().starts_with("bestmove e2e4"));
    assert!(started.elapsed() < Duration::from_secs(1));
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}
#[test]
fn aggression_option_is_snapshotted_clamped_and_persistent() {
    let mut engine = EngineProcess::spawn();
    let position =
        "position fen 2rq1rk1/1p3ppp/p1n1bn2/3pp3/3P4/2P1PN1P/PP1NBPP1/2RQ1RK1 w - - 0 12";

    engine.send(position);
    engine.send("go nodes 20000");
    let started = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("info depth "));
    assert!(started.iter().any(|line| line.starts_with("info depth ")));
    engine.send("setoption name Aggression value -999999999999999999999999999999");
    let active = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    engine.send("ucinewgame");
    engine.send(position);
    engine.send("go nodes 20000");
    let base = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));
    assert!(
        base.last()
            .is_some_and(|line| line.starts_with("bestmove "))
    );

    engine.send("setoption name Aggression value 75");
    engine.send("ucinewgame");
    engine.send(position);
    engine.send("go nodes 20000");
    let default_profile = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));
    assert_eq!(active.last(), default_profile.last());

    engine.send("setoption name Aggression value 999999999999999999999999999999");
    engine.send("ucinewgame");
    engine.send(position);
    engine.send("go nodes 20000");
    let tuned = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));
    assert!(
        tuned
            .last()
            .is_some_and(|line| line.starts_with("bestmove "))
    );

    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}
#[test]
fn end_of_input_cancels_search_without_a_bestmove() {
    let mut engine = EngineProcess::spawn();
    engine.send("position startpos");
    engine.send("go infinite");
    engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("info depth "));

    engine.close_input();
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
    let remaining = engine.receive_to_end(TEST_TIMEOUT);

    assert!(remaining.iter().all(|line| !line.starts_with("bestmove ")));
}
#[test]
fn quit_cancels_active_search_without_a_bestmove() {
    let mut engine = EngineProcess::spawn();
    engine.send("position startpos");
    engine.send("go infinite");
    engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("info depth "));

    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
    let remaining = engine.receive_to_end(TEST_TIMEOUT);

    assert!(remaining.iter().all(|line| !line.starts_with("bestmove ")));
}

#[test]
fn rejected_position_does_not_replace_engine_state() {
    let mut engine = EngineProcess::spawn();
    engine.send("position startpos moves e2e4");
    engine.send("position startpos moves e2e5");
    engine.send("go searchmoves e7e5 depth 1");

    let result = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    assert!(result.last().unwrap().starts_with("bestmove e7e5"));
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}

#[test]
fn terminal_position_returns_the_null_move() {
    let mut engine = EngineProcess::spawn();
    engine.send("position fen 7k/6Q1/6K1/8/8/8/8/8 b - - 0 1");
    engine.send("go depth 1");

    let result = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    assert_eq!(result.last().unwrap(), "bestmove 0000");
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}
#[test]
fn forced_mate_is_reported_in_uci_moves() {
    let mut engine = EngineProcess::spawn();
    engine.send("position fen 7k/5Q2/6K1/8/8/8/8/8 w - - 0 1");
    engine.send("go depth 1");

    let result = engine.receive_until(TEST_TIMEOUT, |line| line.starts_with("bestmove "));

    assert!(result.iter().any(|line| line.contains(" score mate 1 ")));
    assert_ne!(result.last().unwrap(), "bestmove 0000");
    engine.send("quit");
    assert!(engine.wait_for_exit(TEST_TIMEOUT).success());
}

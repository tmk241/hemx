use hemx_test::TestProcess;
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn process_harness_reports_early_exit_with_context() {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .arg("--exact")
        .arg("helper_process_exits_successfully")
        .arg("--nocapture");

    let error = match TestProcess::start(
        command,
        "short-lived helper",
        "127.0.0.1:9",
        Duration::from_secs(2),
    ) {
        Ok(_) => panic!("a process that exits before readiness must fail startup"),
        Err(error) => error,
    };
    let message = error.to_string();

    assert!(message.contains("short-lived helper"), "{message}");
    assert!(message.contains("127.0.0.1:9"), "{message}");
    assert!(message.contains("exited with"), "{message}");
}

#[test]
fn process_harness_waits_for_readiness_and_reaps_on_drop() {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = reservation.local_addr().unwrap().to_string();
    drop(reservation);

    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg("helper_process_listens")
        .arg("--nocapture")
        .env("HEMX_TEST_PROCESS_ADDR", &addr);

    let process = TestProcess::start(command, "listening helper", &addr, Duration::from_secs(2))
        .expect("readiness must observe the helper listener");
    assert!(TcpStream::connect(&addr).is_ok());
    drop(process);

    let deadline = Instant::now() + Duration::from_secs(2);
    while TcpStream::connect(&addr).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        TcpStream::connect(&addr).is_err(),
        "drop must reap the helper"
    );
}

#[test]
fn process_harness_reports_spawn_and_readiness_timeout_errors() {
    let spawn_error = match TestProcess::start(
        Command::new("/definitely/not/a/hemx/executable"),
        "missing helper",
        "127.0.0.1:9",
        Duration::from_millis(10),
    ) {
        Ok(_) => panic!("spawn failure must be returned, not panic"),
        Err(error) => error,
    };
    assert!(spawn_error
        .to_string()
        .contains("failed to spawn missing helper"));

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let unused_addr = reservation.local_addr().unwrap().to_string();
    drop(reservation);
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg("helper_process_sleeps")
        .arg("--nocapture")
        .env("HEMX_TEST_PROCESS_SLEEP", "1");
    let timeout = match TestProcess::start(
        command,
        "non-listening helper",
        &unused_addr,
        Duration::from_millis(100),
    ) {
        Ok(_) => panic!("non-listening process must time out"),
        Err(error) => error,
    };
    let message = timeout.to_string();
    assert!(message.contains("timed out"), "{message}");
    assert!(message.contains("non-listening helper"), "{message}");
}

#[test]
fn helper_process_exits_successfully() {}

#[test]
fn helper_process_listens() {
    let Ok(addr) = std::env::var("HEMX_TEST_PROCESS_ADDR") else {
        return;
    };
    let _listener = TcpListener::bind(addr).expect("bind helper listener");
    std::thread::sleep(Duration::from_secs(10));
}

#[test]
fn helper_process_sleeps() {
    if std::env::var_os("HEMX_TEST_PROCESS_SLEEP").is_some() {
        std::thread::sleep(Duration::from_secs(10));
    }
}

use hemx_test::{ProcessError, TestProcess};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

fn current_test_command(helper: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command.arg("--exact").arg(helper).arg("--nocapture");
    command
}

fn unused_loopback_addr() -> String {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap().to_string();
    drop(reservation);
    address
}

fn wait_until_closed(address: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while TcpStream::connect(address).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        TcpStream::connect(address).is_err(),
        "child listener {address} must be closed"
    );
}

#[test]
fn builder_waits_for_delayed_tcp_readiness_and_captures_output() {
    let address = unused_loopback_addr();
    let process = TestProcess::builder(current_test_command("helper_process_listens"))
        .label("delayed TCP helper")
        .env("HEMX_TEST_PROCESS_ADDR", &address)
        .env("HEMX_TEST_PROCESS_DELAY_MS", "75")
        .tcp(&address)
        .timeout(Duration::from_secs(2))
        .poll_interval(Duration::from_millis(10))
        .start()
        .expect("readiness must observe the helper listener");

    assert!(process.id().is_some());
    assert!(TcpStream::connect(&address).is_ok());
    assert!(process.stdout().contains("tcp helper ready"));
    drop(process);
    wait_until_closed(&address);
}

#[test]
fn builder_waits_for_successful_http_readiness() {
    let address = unused_loopback_addr();
    let process = TestProcess::builder(current_test_command("helper_process_serves_http"))
        .label("HTTP helper")
        .env("HEMX_TEST_PROCESS_ADDR", &address)
        .http(&address, "/health")
        .timeout(Duration::from_secs(2))
        .poll_interval(Duration::from_millis(10))
        .start()
        .expect("the second health response is successful");

    assert!(process.stdout().contains("http helper ready"));
    drop(process);
    wait_until_closed(&address);
}

#[test]
fn early_exit_reports_bounded_stdout_and_stderr() {
    let error = TestProcess::builder(current_test_command("helper_process_is_noisy"))
        .label("noisy helper")
        .env("HEMX_TEST_PROCESS_NOISY", "1")
        .tcp("127.0.0.1:9")
        .output_limit(256)
        .timeout(Duration::from_secs(2))
        .start()
        .unwrap_err();
    let message = error.to_string();

    assert!(matches!(error, ProcessError::EarlyExit { .. }));
    assert!(message.contains("noisy helper"), "{message}");
    assert!(message.contains("exited with"), "{message}");
    assert!(message.contains("earlier bytes omitted"), "{message}");
    assert!(message.contains("stdout marker"), "{message}");
    assert!(message.contains("stderr marker"), "{message}");
    assert!(
        message.len() < 1_500,
        "diagnostic was not bounded: {message}"
    );
}

#[test]
fn timeout_reports_readiness_attempts_output_and_cleanup() {
    let address = unused_loopback_addr();
    let error = TestProcess::builder(current_test_command("helper_process_sleeps"))
        .label("non-listening helper")
        .env("HEMX_TEST_PROCESS_SLEEP", "1")
        .tcp(&address)
        .timeout(Duration::from_millis(100))
        .poll_interval(Duration::from_millis(10))
        .start()
        .unwrap_err();
    let message = error.to_string();

    assert!(matches!(error, ProcessError::TimedOut { .. }));
    assert!(message.contains("timed out"), "{message}");
    assert!(message.contains("non-listening helper"), "{message}");
    assert!(message.contains("readiness attempts"), "{message}");
    assert!(message.contains("sleeping helper started"), "{message}");
}

#[test]
fn configuration_spawn_and_occupied_http_fail_honestly() {
    let missing_readiness = TestProcess::builder(current_test_command("helper_process_sleeps"))
        .label("unconfigured helper")
        .start()
        .unwrap_err();
    assert!(matches!(
        missing_readiness,
        ProcessError::Configuration { .. }
    ));

    let invalid_http = TestProcess::builder(current_test_command("helper_process_sleeps"))
        .label("invalid HTTP helper")
        .http("127.0.0.1:9", "health")
        .start()
        .unwrap_err();
    assert!(invalid_http
        .to_string()
        .contains("path must start with '/'"));

    let spawn = TestProcess::builder(Command::new("/definitely/not/a/hemx/executable"))
        .label("missing helper")
        .tcp("127.0.0.1:9")
        .start()
        .unwrap_err();
    assert!(matches!(spawn, ProcessError::Spawn { .. }));
    assert!(spawn.to_string().contains("failed to spawn missing helper"));

    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = occupied.local_addr().unwrap().to_string();
    let occupied_error = TestProcess::builder(current_test_command("helper_process_sleeps"))
        .label("HTTP ownership helper")
        .env("HEMX_TEST_PROCESS_SLEEP", "1")
        .http(&address, "/health")
        .timeout(Duration::from_millis(100))
        .poll_interval(Duration::from_millis(10))
        .start()
        .unwrap_err();
    assert!(matches!(occupied_error, ProcessError::TimedOut { .. }));
    drop(occupied);
}

#[test]
fn explicit_shutdown_and_drop_after_panic_reap_the_child() {
    let address = unused_loopback_addr();
    let mut process = TestProcess::builder(current_test_command("helper_process_listens"))
        .env("HEMX_TEST_PROCESS_ADDR", &address)
        .tcp(&address)
        .start()
        .unwrap();
    process.shutdown().unwrap();
    process.shutdown().unwrap();
    assert!(process.id().is_none());
    assert!(process.exit_status().is_some());
    wait_until_closed(&address);

    let panic_address = unused_loopback_addr();
    let result = std::panic::catch_unwind(|| {
        let _process = TestProcess::builder(current_test_command("helper_process_listens"))
            .env("HEMX_TEST_PROCESS_ADDR", &panic_address)
            .tcp(&panic_address)
            .start()
            .unwrap();
        panic!("exercise panic cleanup");
    });
    assert!(result.is_err());
    wait_until_closed(&panic_address);
}

#[test]
fn compatibility_start_still_waits_for_tcp_and_reaps() {
    let address = unused_loopback_addr();
    let mut command = current_test_command("helper_process_listens");
    command.env("HEMX_TEST_PROCESS_ADDR", &address);
    let process = TestProcess::start(
        command,
        "compatibility helper",
        &address,
        Duration::from_secs(2),
    )
    .unwrap();
    drop(process);
    wait_until_closed(&address);
}

#[test]
fn helper_process_listens() {
    let Ok(address) = std::env::var("HEMX_TEST_PROCESS_ADDR") else {
        return;
    };
    if let Ok(delay) = std::env::var("HEMX_TEST_PROCESS_DELAY_MS") {
        std::thread::sleep(Duration::from_millis(delay.parse().unwrap()));
    }
    let _listener = TcpListener::bind(address).expect("bind helper listener");
    println!("tcp helper ready");
    std::thread::sleep(Duration::from_secs(10));
}

#[test]
fn helper_process_serves_http() {
    let Ok(address) = std::env::var("HEMX_TEST_PROCESS_ADDR") else {
        return;
    };
    let listener = TcpListener::bind(address).expect("bind HTTP helper listener");
    println!("http helper ready");
    for status in ["503 Service Unavailable", "204 No Content"] {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 512];
        let read = stream.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /health HTTP/1.1"));
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    }
    std::thread::sleep(Duration::from_secs(10));
}

#[test]
fn helper_process_is_noisy() {
    if std::env::var_os("HEMX_TEST_PROCESS_NOISY").is_none() {
        return;
    }
    println!("{}\nstdout marker", "o".repeat(4_096));
    eprintln!("{}\nstderr marker", "e".repeat(4_096));
}

#[test]
fn helper_process_sleeps() {
    if std::env::var_os("HEMX_TEST_PROCESS_SLEEP").is_some() {
        println!("sleeping helper started");
        std::thread::sleep(Duration::from_secs(10));
    }
}

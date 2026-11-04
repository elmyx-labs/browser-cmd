pub(crate) fn stop_owned_daemon(addr: &str) {
    fn matches(command: &str, addr: &str) -> bool {
        let mut words = command.split_whitespace();
        words.next() == Some(env!("CARGO_BIN_EXE_browser-cmd"))
            && words.next() == Some("--serve")
            && words.next() == Some(addr)
            && words.next().is_none()
    }
    let processes =
        std::process::Command::new("ps").args(["-axo", "pid=,command="]).output().expect("list processes");
    assert!(processes.status.success(), "ps failed while cleaning up test daemon");
    let command_line = String::from_utf8_lossy(&processes.stdout);
    let mut owners = command_line.lines().filter_map(|line| {
        let (pid, command) = line.trim().split_once(char::is_whitespace)?;
        matches(command.trim_start(), addr).then(|| pid.parse::<u32>().ok()).flatten()
    });
    let Some(pid) = owners.next() else { return };
    assert!(owners.next().is_none(), "multiple browser-cmd daemons own test address {addr}");

    let verify = || {
        std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| matches(String::from_utf8_lossy(&output.stdout).trim(), addr))
    };
    assert!(verify(), "daemon identity changed before test cleanup");
    assert!(std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status().unwrap().success());
    for _ in 0..40 {
        if !verify() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("owned test daemon {pid} did not stop");
}

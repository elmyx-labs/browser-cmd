//! The standalone browser: a real Chrome for any agent, driven as a shell CLI
//! (`browser-cmd click @e1a.31`) against a warm daemon that holds the tab
//! and its refs between commands. It needs no harness to run — this is the binary.
fn main() {
    std::process::exit(browser_cmd::run());
}

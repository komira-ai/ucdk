// SPDX-License-Identifier: Apache-2.0
//! A program for the tests to run in place of a real tool. A test copies it to a path and writes
//! the file `<path>.behavior` beside the copy; the program reads that file for what to do. The
//! file is lines of words, bytes written as hexadecimal (`-` for none):
//!
//! - `prints <code> <stdout> <stderr>`: writes both, then ends with the code.
//! - `killed <signal>`: ends by the signal it sends itself.
//! - `reports`: writes its working directory, its arguments and its environment, one line each.
//! - `chatters <chunks>`: writes the line `chunk` that many times, each time waiting until the
//!   reader has taken the last one, so that each is read on its own.
//! - `holds <name>`: is called `<name>` in the process table, starts a program called
//!   `<name>.group` in its process group, and waits until it is killed.
//! - `tool` and then `rule <verb> <code> <stdout> <stderr> <report>` and
//!   `file <path> <bytes>` lines: a stand-in for a build tool. On every call it writes the
//!   files, then answers as the rule of the verb it was called with says, and writes `report`
//!   to the file that the flag `--event-log=` or `--build_event_json_file=` names.
//!
//! It is Linux only: it names itself with `prctl`.

use std::ffi::{CString, OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::process::{Command, Stdio, exit};
use std::thread;
use std::time::Duration;

use rustix::process::{Resource, Rlimit, Signal, getpid, kill_process, setrlimit};

fn unhex(text: &str) -> Vec<u8> {
    if text == "-" {
        return Vec::new();
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("hexadecimal"))
        .collect()
}

fn wait_for_ever() -> ! {
    loop {
        thread::park();
    }
}

/// Names this process as `name` in the process table.
fn call_self(name: &str) {
    let name = CString::new(name).expect("a name with no NUL");
    rustix::thread::set_name(&name).expect("the name is set");
}

fn write_all(bytes: &[u8], stdout: bool) {
    if stdout {
        let mut out = std::io::stdout();
        out.write_all(bytes).expect("write");
        out.flush().expect("flush");
    } else {
        let mut err = std::io::stderr();
        err.write_all(bytes).expect("write");
        err.flush().expect("flush");
    }
}

fn report() {
    let mut text: Vec<u8> = b"cwd ".to_vec();
    text.extend(std::env::current_dir().expect("cwd").as_os_str().as_bytes());
    text.push(b'\n');
    for arg in std::env::args_os().skip(1) {
        text.extend(b"arg ");
        text.extend(arg.as_bytes());
        text.push(b'\n');
    }
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    vars.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    for (key, value) in vars {
        text.extend(b"env ");
        text.extend(key.as_bytes());
        text.push(b'=');
        text.extend(value.as_bytes());
        text.push(b'\n');
    }
    write_all(&text, true);
}

fn chatter(chunks: u32) {
    for _ in 0..chunks {
        write_all(b"chunk\n", true);
        // The pipe is empty once the reader has taken the chunk.
        while rustix::io::ioctl_fionread(std::io::stdout()).unwrap_or(0) > 0 {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

// The grandchild is left to be orphaned: the test is of what a stop does to it.
#[allow(clippy::zombie_processes)]
fn hold(name: &str) -> ! {
    call_self(name);
    let exe = std::env::current_exe().expect("own path");
    Command::new(exe)
        .arg("--group")
        .arg(format!("{name}.group"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the grandchild starts");
    wait_for_ever()
}

fn kill_self(signal: i32) -> ! {
    // A signal that dumps core leaves no file.
    let none = Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    setrlimit(Resource::Core, none).expect("no core file");
    let signal = Signal::from_named_raw(signal).expect("a signal");
    kill_process(getpid(), signal).expect("the signal is sent");
    wait_for_ever()
}

/// The words of the line of `lines` that starts with `first` (and `second`, when given).
fn line<'a>(
    lines: &'a [Vec<&'a str>],
    first: &str,
    second: Option<&str>,
) -> Option<&'a Vec<&'a str>> {
    lines
        .iter()
        .find(|words| words[0] == first && second.is_none_or(|s| words.get(1) == Some(&s)))
}

fn tool(lines: &[Vec<&str>]) -> ! {
    for words in lines.iter().filter(|words| words[0] == "file") {
        let path = OsString::from_vec(unhex(words[1]));
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(parent).expect("directories");
        }
        std::fs::write(&path, unhex(words[2])).expect("file");
    }
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let verb = args
        .iter()
        .filter_map(|arg| arg.to_str())
        .find(|arg| ["build", "test", "cquery", "info"].contains(arg))
        .expect("a verb");
    let rule = line(lines, "rule", Some(verb)).expect("a rule for the verb");
    for arg in &args {
        for flag in ["--event-log=", "--build_event_json_file="] {
            if let Some(path) = arg.as_bytes().strip_prefix(flag.as_bytes()) {
                std::fs::write(OsStr::from_bytes(path), unhex(rule[5])).expect("report");
            }
        }
    }
    write_all(&unhex(rule[3]), true);
    write_all(&unhex(rule[4]), false);
    exit(rule[2].parse().expect("a code"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, name] = args.as_slice()
        && flag == "--group"
    {
        call_self(name);
        wait_for_ever();
    }
    let mut sidecar = std::env::current_exe().expect("own path").into_os_string();
    sidecar.push(".behavior");
    let text = std::fs::read_to_string(sidecar).expect("the file beside the program");
    let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split(' ').collect()).collect();
    let first = &lines[0];
    match first[0] {
        "prints" => {
            write_all(&unhex(first[2]), true);
            write_all(&unhex(first[3]), false);
            exit(first[1].parse().expect("a code"));
        }
        "killed" => kill_self(first[1].parse().expect("a signal")),
        "reports" => report(),
        "chatters" => chatter(first[1].parse().expect("a count")),
        "holds" => hold(first[1]),
        "tool" => tool(&lines),
        other => panic!("no behavior called {other}"),
    }
}

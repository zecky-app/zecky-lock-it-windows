//! Starts Go Max with real node programs, reads the status, and stops it.
//!
//! ```text
//! go_max_smoke <zakurad> <zainod> <root folder> [seconds to run] [rounds]
//! ```
//!
//! `GO_MAX_INITIAL_PEERS=a:port,b:port` replaces the seed list.
//!
//! With `zainod` given as `-`, only Zakura is started and stopped (useful
//! where no Zaino build is at hand). Each round is: start, wait, read the
//! status, stop. The output says whether every stop was clean.

use std::{
    ffi::OsStr,
    path::PathBuf,
    time::{Duration, Instant},
};

use lockit_node::{
    process::{self, NodeProcess, ProcessGroup, STOP_GRACE},
    GoMax, GoMaxSettings, NodeLayout, NodePrograms, NodeRpc,
};

fn main() {
    if let Some(code) = process::run_stop_helper_if_asked() {
        std::process::exit(code);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: go_max_smoke <zakurad> <zainod|-> <root> [seconds] [rounds]");
        std::process::exit(2);
    }
    let zakurad = PathBuf::from(&args[0]);
    let zainod = (args[1] != "-").then(|| PathBuf::from(&args[1]));
    let layout = NodeLayout::new(&args[2]);
    let seconds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    let rounds: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut settings = GoMaxSettings::default();
    if let Ok(peers) = std::env::var("GO_MAX_INITIAL_PEERS") {
        settings.initial_peers = Some(peers.split(',').map(str::to_owned).collect());
    }

    let mut all_clean = true;
    for round in 1..=rounds {
        println!("== round {round} of {rounds}");
        let clean = match &zainod {
            Some(zainod) => full(&layout, &settings, &zakurad, zainod, seconds),
            None => zakura_only(&layout, &settings, &zakurad, seconds),
        };
        all_clean &= clean;
    }
    println!("RESULT {}", if all_clean { "every stop was clean" } else { "at least one stop was NOT clean" });
    std::process::exit(if all_clean { 0 } else { 1 });
}

fn full(layout: &NodeLayout, settings: &GoMaxSettings, zakurad: &PathBuf, zainod: &PathBuf, seconds: u64) -> bool {
    let programs = NodePrograms { zakurad: zakurad.clone(), zainod: zainod.clone() };
    let mut go_max = GoMax::new(layout.clone(), settings.clone(), programs);
    let started = Instant::now();
    if let Err(error) = go_max.start(|step| println!("   step: {step}")) {
        println!("   start FAILED after {} s: {error}", started.elapsed().as_secs());
        return false;
    }
    println!("   started in {} s", started.elapsed().as_secs());
    watch(seconds, || {
        go_max
            .status()
            .map(|s| format!("{s:?}, wallet port open: {}", go_max.wallet_ready()))
            .map_err(|e| e.to_string())
    });
    match go_max.stop() {
        Ok(report) => {
            println!("   stop: {report:?}");
            report.clean()
        }
        Err(error) => {
            println!("   stop FAILED: {error}");
            false
        }
    }
}

fn zakura_only(layout: &NodeLayout, settings: &GoMaxSettings, zakurad: &PathBuf, seconds: u64) -> bool {
    layout.create().expect("folders");
    settings.write_configs(layout).expect("configs");
    let _ = std::fs::remove_file(layout.cookie_file());
    let group = ProcessGroup::new().expect("group");
    let config = layout.zakura_config();
    let started = Instant::now();
    let mut zakura = match NodeProcess::start(
        &group,
        "Zakura",
        zakurad,
        &[OsStr::new("-c"), config.as_os_str(), OsStr::new("start")],
        &layout.log_file("zakurad"),
    ) {
        Ok(process) => process,
        Err(error) => {
            println!("   start FAILED: {error}");
            return false;
        }
    };
    let rpc = NodeRpc::new(settings.rpc_address(), layout.cookie_file());
    let deadline = Instant::now() + Duration::from_secs(300);
    while !rpc.is_ready() {
        if let Err(error) = zakura.check_running() {
            println!("   Zakura exited: {error}");
            return false;
        }
        if Instant::now() > deadline {
            println!("   Zakura did not answer RPC in time");
            let _ = zakura.stop(STOP_GRACE);
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("   Zakura answers RPC after {} s", started.elapsed().as_secs());
    watch(seconds, || rpc.status().map(|s| format!("{s:?}")).map_err(|e| e.to_string()));
    let stopping = Instant::now();
    match zakura.stop(STOP_GRACE) {
        Ok(clean) => {
            println!("   stop: clean={clean} in {} s", stopping.elapsed().as_secs());
            clean
        }
        Err(error) => {
            println!("   stop FAILED: {error}");
            false
        }
    }
}

fn watch(seconds: u64, mut status: impl FnMut() -> Result<String, String>) {
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        match status() {
            Ok(text) => println!("   status: {text}"),
            Err(error) => println!("   status error: {error}"),
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

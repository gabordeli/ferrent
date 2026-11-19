#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

#[tauri::command]
fn report_startup(ms: f64) {
    let startup = START
        .get()
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    println!("startup_ms={startup:.1} js_first_frame_ms={ms:.1}");
}

#[tauri::command]
fn report_metrics(json: String) {
    println!("metrics_json={json}");
    println!("rss_kb={}", rss_tree_kb());
    println!("framework=tauri");
    std::process::exit(0);
}

fn rss_tree_kb() -> u64 {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-eo", "pid=,ppid=,rss="])
        .output()
    else {
        return 0;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(pid), Some(ppid), Some(rss)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Ok(pid), Ok(ppid), Ok(rss)) =
            (pid.parse::<i32>(), ppid.parse::<i32>(), rss.parse::<u64>())
        else {
            continue;
        };
        processes.push((pid, ppid, rss));
    }

    let mut included = HashSet::from([std::process::id() as i32]);
    loop {
        let before = included.len();
        for (pid, ppid, _) in &processes {
            if included.contains(ppid) {
                included.insert(*pid);
            }
        }
        if included.len() == before {
            break;
        }
    }

    processes
        .iter()
        .filter(|(pid, _, _)| included.contains(pid))
        .map(|(_, _, rss)| rss)
        .sum()
}

fn main() {
    START.set(Instant::now()).ok();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![report_startup, report_metrics])
        .run(tauri::generate_context!())
        .expect("run Tauri spike");
}

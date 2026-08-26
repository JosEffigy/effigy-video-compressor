use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as AsyncCommand;
use tokio::sync::oneshot;

// ── Windows: suppress console windows ────────────────────────────────────────
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::GetCurrentProcess,
    },
};
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn hidden_cmd(program: &str) -> AsyncCommand {
    let mut cmd = AsyncCommand::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn hidden_std_cmd(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

// ── App state (resolved binary paths, set once by check_ffmpeg) ──────────────
pub struct AppState {
    pub ffmpeg: Mutex<Option<String>>,
    pub ffprobe: Mutex<Option<String>>,
    pub active_pids: Mutex<HashMap<usize, u32>>,
    pub cancelled_files: Mutex<HashSet<usize>>,
    pub cancel_all: AtomicBool,
    pub running: AtomicBool,
    pub allow_close: AtomicBool,
    pub minimize_to_tray: AtomicBool,
    #[cfg(windows)]
    pub process_job: ProcessJob,
}

#[cfg(windows)]
pub struct ProcessJob {
    handle: HANDLE,
    contains_app: bool,
}

#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
unsafe impl Sync for ProcessJob {}

#[cfg(windows)]
impl ProcessJob {
    fn new() -> Self {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !handle.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let _ = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32,
                );
            }
            // Put the application itself in the job before Tauri creates its
            // WebView processes. Children then inherit the kill-on-close job,
            // so an End Task or crash cannot orphan them. Explicit assignment
            // is still used for encoder workers as a fallback.
            let contains_app =
                !handle.is_null() && AssignProcessToJobObject(handle, GetCurrentProcess()) != 0;
            Self {
                handle,
                contains_app,
            }
        }
    }

    fn assign(&self, child: &tokio::process::Child) {
        if self.handle.is_null() {
            return;
        }
        unsafe {
            if let Some(process) = child.raw_handle() {
                let _ = AssignProcessToJobObject(self.handle, process as HANDLE);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        if !self.handle.is_null() && !self.contains_app {
            unsafe {
                CloseHandle(self.handle);
            }
        }
        // If the app belongs to the job, keep this handle alive until Windows
        // tears down the process. Closing it here would terminate the app while
        // Tauri is still finishing its normal shutdown sequence.
    }
}

fn is_cancelled(state: &AppState, file_index: usize) -> bool {
    state.cancel_all.load(Ordering::SeqCst)
        || state.cancelled_files.lock().unwrap().contains(&file_index)
}

async fn terminate_pid(pid: u32) {
    #[cfg(windows)]
    {
        let _ = hidden_cmd("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    #[cfg(not(windows))]
    {
        let _ = hidden_cmd("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .await;
    }
}

struct FfmpegRun<'a> {
    ffmpeg: &'a str,
    file_index: usize,
    file_name: &'a str,
    args: &'a [String],
    duration: f64,
    progress_start: f64,
    progress_span: f64,
    stage: &'a str,
}

async fn run_ffmpeg_process(
    state: &AppState,
    app: &AppHandle,
    run: FfmpegRun<'_>,
) -> Result<std::process::ExitStatus, String> {
    let mut child = hidden_cmd(run.ffmpeg)
        .args(run.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to launch FFmpeg: {e}"))?;

    #[cfg(windows)]
    state.process_job.assign(&child);
    if let Some(pid) = child.id() {
        state
            .active_pids
            .lock()
            .unwrap()
            .insert(run.file_index, pid);
    }

    if let Some(stderr) = child.stderr.take() {
        let app_log = app.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app_log.emit("ffmpeg-log", LogEvent { line });
            }
        });
    }

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(t) = line.strip_prefix("out_time=") {
                if let Some(secs) = parse_ffmpeg_time(t) {
                    let local = (secs / run.duration).clamp(0.0, 1.0);
                    let percent = (run.progress_start + local * run.progress_span).clamp(0.0, 99.0);
                    let _ = app.emit(
                        "progress",
                        ProgressEvent {
                            file_index: run.file_index,
                            file_name: run.file_name.to_string(),
                            percent,
                            stage: format!("{} {percent:.0}%", run.stage),
                        },
                    );
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("FFmpeg wait error: {e}"));
    state.active_pids.lock().unwrap().remove(&run.file_index);
    status
}

fn cleanup_passlogs(prefix: &std::path::Path) {
    let Some(parent) = prefix.parent() else {
        return;
    };
    let Some(name) = prefix.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn add_two_pass_args(args: &mut Vec<String>, codec: &str, pass: u8, prefix: &std::path::Path) {
    if codec == "libx265" {
        let mut stats = prefix.to_string_lossy().replace('\\', "/");
        if cfg!(windows) && stats.len() > 1 && stats.as_bytes()[1] == b':' {
            stats.insert(1, '\\');
        }
        if let Some(index) = args.iter().position(|arg| arg == "-x265-params") {
            if let Some(params) = args.get_mut(index + 1) {
                params.push_str(&format!(":pass={pass}:stats={stats}"));
            }
        }
    } else {
        args.extend([
            "-pass".into(),
            pass.to_string(),
            "-passlogfile".into(),
            prefix.to_string_lossy().to_string(),
        ]);
    }
}

fn set_ffmpeg_arg(args: &mut [String], flag: &str, value: String) -> bool {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return false;
    };
    let Some(existing) = args.get_mut(index + 1) else {
        return false;
    };
    *existing = value;
    true
}

fn corrected_video_bitrate(current_kbps: i64, target_bytes: u64, actual_bytes: u64) -> i64 {
    if actual_bytes == 0 {
        return current_kbps.max(1);
    }
    let ratio = target_bytes as f64 / actual_bytes as f64;
    ((current_kbps as f64 * ratio * 0.98).floor() as i64).max(1)
}

// ── Binary resolution ─────────────────────────────────────────────────────────
// Search order:
//   1. Next to the running .exe  (manual drop-in)
//   2. %LOCALAPPDATA%\Programs\FFmpeg\bin\  (auto-install destination)
//   3. System PATH

fn resolve_binary_sync(name: &str) -> Option<(String, &'static str)> {
    // 1. Beside the exe
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            #[cfg(windows)]
            let p = dir.join(format!("{}.exe", name));
            #[cfg(not(windows))]
            let p = dir.join(name);
            if p.exists() {
                return Some((p.to_string_lossy().to_string(), "local"));
            }
        }
    }

    // 2. Neutral per-user install folder (does not require elevation).
    #[cfg(windows)]
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        let p = std::path::Path::new(&la)
            .join("Programs")
            .join("FFmpeg")
            .join("bin")
            .join(format!("{}.exe", name));
        if p.exists() {
            return Some((p.to_string_lossy().to_string(), "local"));
        }
    }

    // 3. System PATH — just try running it
    let found = hidden_std_cmd(name)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if found {
        Some((name.to_string(), "path"))
    } else {
        None
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

struct EncodingBudget {
    video_kbps: i64,
    audio_kbps: u32,
    reserved_kbits: f64,
}

fn adaptive_audio_kbps(container: &str, channels: u32, total_kbps: Option<f64>) -> u32 {
    if channels == 0 {
        return 0;
    }
    let ceiling = if channels == 1 {
        80
    } else if container == "webm" {
        128
    } else {
        112
    };
    let preferred = match total_kbps {
        Some(total) if total < 450.0 => 48,
        Some(total) if total < 900.0 => 64,
        Some(total) if total < 1_800.0 => 80,
        Some(_) => ceiling,
        None => ceiling,
    };
    let share_limited = total_kbps
        .map(|total| (total * 0.16).round() as u32)
        .unwrap_or(ceiling);
    preferred.min(ceiling).min(share_limited.max(32))
}

fn fixed_size_budget(
    duration_secs: f64,
    target_mb: f64,
    container: &str,
    audio_channels: u32,
) -> EncodingBudget {
    let total_kbits = target_mb * 8.0 * 1024.0;
    // Reserve proportional muxing overhead, an absolute metadata allowance,
    // and encoder variance so the result stays below strict upload limits.
    let reserved_kbits = (total_kbits * 0.04).max(128.0);
    let usable_kbits = (total_kbits - reserved_kbits).max(0.0);
    let usable_kbps = usable_kbits / duration_secs.max(0.001);
    let audio_kbps = adaptive_audio_kbps(container, audio_channels, Some(usable_kbps));
    let video_kbps = (usable_kbps - audio_kbps as f64).floor() as i64;
    EncodingBudget {
        video_kbps,
        audio_kbps,
        reserved_kbits,
    }
}

fn parse_ffmpeg_time(s: &str) -> Option<f64> {
    let p: Vec<&str> = s.split(':').collect();
    if p.len() != 3 {
        return None;
    }
    Some(
        p[0].parse::<f64>().ok()? * 3600.0
            + p[1].parse::<f64>().ok()? * 60.0
            + p[2].parse::<f64>().ok()?,
    )
}

#[derive(Clone, Copy)]
struct MediaInfo {
    duration: f64,
    width: u32,
    height: u32,
    fps: f64,
    audio_channels: u32,
    motion: f64,
}

async fn probe_media(state: &AppState, ffprobe: &str, file: &str) -> Result<MediaInfo, String> {
    async fn run_probe(
        state: &AppState,
        ffprobe: &str,
        args: &[&str],
    ) -> Result<std::process::Output, String> {
        let mut command = hidden_cmd(ffprobe);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|e| format!("ffprobe failed: {e}"))?;
        #[cfg(windows)]
        state.process_job.assign(&child);
        child
            .wait_with_output()
            .await
            .map_err(|e| format!("ffprobe failed: {e}"))
    }
    let out = run_probe(
        state,
        ffprobe,
        &[
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,width,height,avg_frame_rate,duration,channels:stream_tags=rotate:stream_side_data=rotation:format=duration",
            "-of",
            "json",
            file,
        ],
    )
    .await?;
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("Invalid FFprobe data: {e}"))?;
    let streams = json
        .get("streams")
        .and_then(|streams| streams.as_array())
        .ok_or_else(|| "FFprobe returned no streams".to_string())?;
    let stream = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(|value| value.as_str()) == Some("video"));
    let parse_number = |value: Option<&serde_json::Value>| {
        value.and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.parse::<f64>().ok())
        })
    };
    let duration = parse_number(json.get("format").and_then(|format| format.get("duration")))
        .or_else(|| parse_number(stream.and_then(|stream| stream.get("duration"))))
        .filter(|duration| *duration > 0.0)
        .ok_or_else(|| "Could not determine duration".to_string())?;
    let encoded_width = stream
        .and_then(|stream| stream.get("width"))
        .and_then(|width| width.as_u64())
        .and_then(|width| u32::try_from(width).ok())
        .filter(|width| *width > 0)
        .unwrap_or(1920);
    let encoded_height = stream
        .and_then(|stream| stream.get("height"))
        .and_then(|height| height.as_u64())
        .and_then(|height| u32::try_from(height).ok())
        .filter(|height| *height > 0)
        .unwrap_or(1080);
    let rotation = stream
        .and_then(|stream| stream.get("side_data_list"))
        .and_then(|list| list.as_array())
        .and_then(|list| {
            list.iter()
                .find_map(|data| parse_number(data.get("rotation")))
        })
        .or_else(|| {
            parse_number(
                stream
                    .and_then(|stream| stream.get("tags"))
                    .and_then(|tags| tags.get("rotate")),
            )
        })
        .unwrap_or(0.0);
    let (width, height) = display_dimensions(encoded_width, encoded_height, rotation);
    let fps = stream
        .and_then(|stream| stream.get("avg_frame_rate"))
        .and_then(|rate| rate.as_str())
        .and_then(|rate| {
            let (numerator, denominator) = rate.split_once('/')?;
            let numerator = numerator.parse::<f64>().ok()?;
            let denominator = denominator.parse::<f64>().ok()?;
            (denominator > 0.0).then_some(numerator / denominator)
        })
        .filter(|fps| fps.is_finite() && *fps > 0.0)
        .unwrap_or(30.0);
    let audio_channels = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(|value| value.as_str()) == Some("audio"))
        .and_then(|stream| stream.get("channels"))
        .and_then(|channels| channels.as_u64())
        .and_then(|channels| u32::try_from(channels).ok())
        .unwrap_or(0);

    Ok(MediaInfo {
        duration,
        width,
        height,
        fps,
        audio_channels,
        motion: 0.0,
    })
}

#[derive(Clone, Copy, Debug)]
struct TimedMetric {
    time: f64,
    value: f64,
}

#[derive(Clone, Debug)]
struct SceneAllocation {
    start: f64,
    end: f64,
    weight: f64,
}

struct TempFileGuard {
    path: std::path::PathBuf,
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn run_analysis_command(
    state: &AppState,
    file_index: usize,
    program: &str,
    args: &[String],
) -> Result<std::process::Output, String> {
    let mut command = hidden_cmd(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("Analysis process failed: {error}"))?;
    #[cfg(windows)]
    state.process_job.assign(&child);
    if let Some(pid) = child.id() {
        state.active_pids.lock().unwrap().insert(file_index, pid);
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|error| format!("Analysis process failed: {error}"));
    state.active_pids.lock().unwrap().remove(&file_index);
    output
}

async fn probe_motion_samples(
    state: &AppState,
    ffmpeg: &str,
    file: &str,
    file_index: usize,
) -> Vec<TimedMetric> {
    let mut command = hidden_cmd(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "info",
            "-i",
            file,
            "-an",
            "-vf",
            "fps=2,scale=320:-2,signalstats,metadata=print:key=lavfi.signalstats.YDIF",
            "-f",
            "null",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let Ok(child) = command.spawn() else {
        return Vec::new();
    };
    #[cfg(windows)]
    state.process_job.assign(&child);
    if let Some(pid) = child.id() {
        state.active_pids.lock().unwrap().insert(file_index, pid);
    }
    let Ok(output) = child.wait_with_output().await else {
        state.active_pids.lock().unwrap().remove(&file_index);
        return Vec::new();
    };
    state.active_pids.lock().unwrap().remove(&file_index);

    let mut time = 0.0;
    let mut samples = Vec::new();
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        if let Some(value) = line.split("pts_time:").nth(1) {
            time = value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(time);
        } else if let Some(value) = line.split("lavfi.signalstats.YDIF=").nth(1) {
            if let Ok(value) = value.trim().parse::<f64>() {
                samples.push(TimedMetric { time, value });
            }
        }
    }
    samples
}

fn mean_motion_complexity(samples: &[TimedMetric]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let average = samples.iter().map(|sample| sample.value).sum::<f64>() / samples.len() as f64;
    (average / 12.0).clamp(0.0, 1.0)
}

async fn probe_quality_packets(
    state: &AppState,
    ffmpeg: &str,
    ffprobe: &str,
    file: &str,
    file_index: usize,
) -> Result<Vec<TimedMetric>, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let proxy = TempFileGuard {
        path: std::env::temp_dir().join(format!(
            "effigy-gameplay-probe-{}-{file_index}-{nonce}.mkv",
            std::process::id()
        )),
    };
    let encode_args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        file.into(),
        "-an".into(),
        "-vf".into(),
        "fps=8,scale=640:640:flags=bilinear:force_original_aspect_ratio=decrease:force_divisible_by=2,format=yuv420p".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "30".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-y".into(),
        proxy.path.to_string_lossy().to_string(),
    ];
    let output = run_analysis_command(state, file_index, ffmpeg, &encode_args).await?;
    if !output.status.success() {
        return Err("Quality probe encode failed".into());
    }

    let probe_args = vec![
        "-v".into(),
        "error".into(),
        "-select_streams".into(),
        "v:0".into(),
        "-show_entries".into(),
        "packet=pts_time,size".into(),
        "-of".into(),
        "json".into(),
        proxy.path.to_string_lossy().to_string(),
    ];
    let output = run_analysis_command(state, file_index, ffprobe, &probe_args).await?;
    if !output.status.success() {
        return Err("Quality probe inspection failed".into());
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Invalid quality probe data: {error}"))?;
    Ok(json
        .get("packets")
        .and_then(|packets| packets.as_array())
        .into_iter()
        .flatten()
        .filter_map(|packet| {
            let time = packet.get("pts_time").and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_str()?.parse::<f64>().ok())
            })?;
            let value = packet.get("size").and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_u64().map(|value| value as f64))
                    .or_else(|| value.as_str()?.parse::<f64>().ok())
            })?;
            Some(TimedMetric { time, value })
        })
        .collect())
}

fn gameplay_allocations(
    duration: f64,
    motion_samples: &[TimedMetric],
    probe_packets: Option<&[TimedMetric]>,
) -> Vec<SceneAllocation> {
    let window = (duration / 300.0).max(4.0);
    let count = (duration / window).ceil().max(1.0) as usize;
    let mut motion_sum = vec![0.0; count];
    let mut motion_count = vec![0_u32; count];
    let mut probe_sum = vec![0.0; count];
    for sample in motion_samples {
        let index = ((sample.time / window).floor() as usize).min(count - 1);
        motion_sum[index] += sample.value;
        motion_count[index] += 1;
    }
    for (sum, count) in motion_sum.iter_mut().zip(motion_count) {
        if count > 0 {
            *sum /= count as f64;
        }
    }
    if let Some(packets) = probe_packets {
        for packet in packets {
            let index = ((packet.time / window).floor() as usize).min(count - 1);
            probe_sum[index] += packet.value;
        }
    }
    let positive_mean = |values: &[f64]| {
        let values = values
            .iter()
            .copied()
            .filter(|value| *value > 0.0)
            .collect::<Vec<_>>();
        if values.is_empty() {
            1.0
        } else {
            values.iter().sum::<f64>() / values.len() as f64
        }
    };
    let motion_mean = positive_mean(&motion_sum);
    let probe_mean = positive_mean(&probe_sum);
    let mut allocations = (0..count)
        .map(|index| {
            let motion_ratio = (motion_sum[index] / motion_mean).clamp(0.2, 3.0).sqrt();
            let raw = if probe_packets.is_some() {
                let probe_ratio = (probe_sum[index] / probe_mean).clamp(0.2, 3.0).sqrt();
                0.45 + 0.20 * motion_ratio + 0.35 * probe_ratio
            } else {
                0.65 + 0.35 * motion_ratio
            };
            SceneAllocation {
                start: index as f64 * window,
                end: ((index + 1) as f64 * window).min(duration),
                weight: raw,
            }
        })
        .collect::<Vec<_>>();
    let weighted_mean = allocations
        .iter()
        .map(|allocation| allocation.weight * (allocation.end - allocation.start))
        .sum::<f64>()
        / duration.max(0.001);
    for allocation in &mut allocations {
        allocation.weight = (allocation.weight / weighted_mean).clamp(0.75, 1.35);
    }
    let corrected_mean = allocations
        .iter()
        .map(|allocation| allocation.weight * (allocation.end - allocation.start))
        .sum::<f64>()
        / duration.max(0.001);
    for allocation in &mut allocations {
        allocation.weight /= corrected_mean;
    }
    allocations
}

fn encoder_zones(allocations: &[SceneAllocation], fps: f64) -> String {
    let mut zones: Vec<(u64, u64, f64)> = Vec::new();
    for allocation in allocations {
        let start = (allocation.start * fps).floor().max(0.0) as u64;
        let end = ((allocation.end * fps).ceil() as u64)
            .saturating_sub(1)
            .max(start);
        let weight = (allocation.weight * 20.0).round() / 20.0;
        if let Some(last) = zones.last_mut() {
            if (last.2 - weight).abs() < 0.001 && last.1 + 1 == start {
                last.1 = end;
                continue;
            }
        }
        zones.push((start, end, weight));
    }
    zones
        .iter()
        .map(|(start, end, weight)| format!("{start},{end},b={weight:.2}"))
        .collect::<Vec<_>>()
        .join("/")
}

struct OutputProfile {
    width: Option<u32>,
    height: Option<u32>,
    resolution_name: String,
    fps: f64,
    fps_name: String,
}

struct ResolutionCandidate {
    width: Option<u32>,
    height: Option<u32>,
    name: String,
    rendered_width: u32,
    rendered_height: u32,
}

fn display_dimensions(width: u32, height: u32, rotation_degrees: f64) -> (u32, u32) {
    let normalized = (rotation_degrees.round() as i32).rem_euclid(180);
    if normalized == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

fn oriented_resolution_box(
    source_width: u32,
    source_height: u32,
    landscape_width: u32,
    landscape_height: u32,
) -> (u32, u32) {
    if source_height > source_width {
        (landscape_height, landscape_width)
    } else {
        (landscape_width, landscape_height)
    }
}

fn scaled_dimensions(
    source_width: u32,
    source_height: u32,
    box_width: u32,
    box_height: u32,
) -> (u32, u32) {
    let scale = (box_width as f64 / source_width as f64)
        .min(box_height as f64 / source_height as f64)
        .min(1.0);
    let even = |value: f64| ((value.floor() as u32).max(2) / 2) * 2;
    (
        even(source_width as f64 * scale),
        even(source_height as f64 * scale),
    )
}

fn auto_bpp_floor(codec: &str) -> f64 {
    let base = if codec == "libsvtav1" || codec.starts_with("av1_") {
        0.035
    } else if codec == "libx265" || codec.starts_with("hevc_") {
        0.045
    } else {
        0.065
    };
    let hardware_factor = if codec.ends_with("_amf") {
        1.12
    } else if codec.ends_with("_qsv") {
        1.08
    } else if codec.ends_with("_nvenc") {
        1.05
    } else {
        1.0
    };
    base * hardware_factor
}

fn select_output_profile(
    options: &CompressOptions,
    media: MediaInfo,
    bitrate_kbps: i64,
) -> OutputProfile {
    let auto_resolution = options.res_mode == "auto";
    let auto_fps = options.fps_name == "auto";

    let mut resolutions: Vec<ResolutionCandidate> = Vec::new();
    if auto_resolution {
        resolutions.push(ResolutionCandidate {
            width: None,
            height: None,
            name: "original".into(),
            rendered_width: media.width,
            rendered_height: media.height,
        });
        for (width, height, name) in [
            (2560, 1440, "1440p"),
            (1920, 1080, "1080p"),
            (1280, 720, "720p"),
            (960, 540, "540p"),
        ] {
            let (width, height) = oriented_resolution_box(media.width, media.height, width, height);
            let (scaled_width, scaled_height) =
                scaled_dimensions(media.width, media.height, width, height);
            if !resolutions.iter().any(|profile| {
                profile.rendered_width == scaled_width && profile.rendered_height == scaled_height
            }) {
                resolutions.push(ResolutionCandidate {
                    width: Some(width),
                    height: Some(height),
                    name: name.into(),
                    rendered_width: scaled_width,
                    rendered_height: scaled_height,
                });
            }
        }
    } else if options.res_mode == "scale" || options.res_mode == "auto" {
        let width = options.res_w.unwrap_or(1920);
        let height = options.res_h.unwrap_or(1080);
        let (width, height) = oriented_resolution_box(media.width, media.height, width, height);
        let (scaled_width, scaled_height) =
            scaled_dimensions(media.width, media.height, width, height);
        resolutions.push(ResolutionCandidate {
            width: Some(width),
            height: Some(height),
            name: if options.res_mode == "auto" {
                "1080p".into()
            } else {
                options.resolution_name.clone()
            },
            rendered_width: scaled_width,
            rendered_height: scaled_height,
        });
    } else {
        resolutions.push(ResolutionCandidate {
            width: None,
            height: None,
            name: options.resolution_name.clone(),
            rendered_width: media.width,
            rendered_height: media.height,
        });
    }

    let frame_rates = if auto_fps {
        if media.fps >= 59.0 {
            vec![60.0, 30.0]
        } else if media.fps >= 29.0 {
            vec![30.0]
        } else {
            vec![media.fps]
        }
    } else {
        vec![if options.fps_name == "auto" {
            60.0_f64.min(media.fps.max(1.0))
        } else {
            options.fps.max(1.0)
        }]
    };

    let required_bpp = auto_bpp_floor(&options.codec) * (1.0 + media.motion * 0.45);
    let mut candidates = resolutions
        .iter()
        .flat_map(|resolution| {
            frame_rates.iter().map(move |fps| {
                let pixel_rate =
                    resolution.rendered_width as f64 * resolution.rendered_height as f64 * fps;
                let bpp = bitrate_kbps.max(1) as f64 * 1000.0 / pixel_rate.max(1.0);
                let score = resolution.rendered_width as f64
                    * resolution.rendered_height as f64
                    * (*fps / 30.0).powf(0.65);
                (resolution, *fps, pixel_rate, bpp, score)
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.4.total_cmp(&left.4));
    let selected = if options.algorithm == "fixed" {
        candidates
            .iter()
            .find(|candidate| candidate.3 >= required_bpp)
            .or_else(|| {
                candidates
                    .iter()
                    .min_by(|left, right| left.2.total_cmp(&right.2))
            })
    } else {
        // CRF has no hard bitrate budget; keep the best source-limited preset.
        candidates.first()
    }
    .expect("at least one output profile");
    let resolution = selected.0;
    let fps = selected.1;
    let fps_name = if (fps - fps.round()).abs() < 0.01 {
        format!("{}fps", fps.round() as u32)
    } else {
        format!("{fps:.2}fps")
    };

    OutputProfile {
        width: resolution.width,
        height: resolution.height,
        resolution_name: resolution.name.clone(),
        fps,
        fps_name,
    }
}

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompressOptions {
    pub codec: String,
    pub codec_name: String,
    pub crf: u32,
    pub quality_mode: String,
    pub gpu_gameplay_mode: String,
    pub container: String,
    pub algorithm: String,
    pub res_mode: String,
    pub res_w: Option<u32>,
    pub res_h: Option<u32>,
    pub resolution_name: String,
    pub fps: f64,
    pub fps_name: String,
    pub target_size: f64,
    pub size_name: String,
    pub allocation_mode: String,
    pub preset: String,
    pub svt_preset: u32,
    pub output_folder: Option<String>,
}

#[cfg(test)]
mod auto_profile_tests {
    use super::*;

    fn options(codec: &str) -> CompressOptions {
        CompressOptions {
            codec: codec.into(),
            codec_name: codec.into(),
            crf: 24,
            quality_mode: "crf".into(),
            gpu_gameplay_mode: "native".into(),
            container: "mp4".into(),
            algorithm: "fixed".into(),
            res_mode: "auto".into(),
            res_w: None,
            res_h: None,
            resolution_name: "auto".into(),
            fps: 0.0,
            fps_name: "auto".into(),
            target_size: 8.0,
            size_name: "8mb".into(),
            allocation_mode: "lightweight".into(),
            preset: "medium".into(),
            svt_preset: 8,
            output_folder: None,
        }
    }

    #[test]
    fn auto_does_not_upscale_small_sources() {
        let media = MediaInfo {
            duration: 30.0,
            width: 1280,
            height: 720,
            fps: 60.0,
            audio_channels: 2,
            motion: 0.0,
        };
        let selected = select_output_profile(&options("libx264"), media, 20_000);
        assert_eq!(selected.resolution_name, "original");
        assert_eq!(selected.fps, 60.0);
    }

    #[test]
    fn portrait_manual_resolution_uses_the_short_side() {
        let media = MediaInfo {
            duration: 30.0,
            width: 2160,
            height: 3840,
            fps: 30.0,
            audio_channels: 2,
            motion: 0.0,
        };
        let mut selected_options = options("libx264");
        selected_options.res_mode = "scale".into();
        selected_options.res_w = Some(1920);
        selected_options.res_h = Some(1080);
        selected_options.resolution_name = "1080p".into();

        let selected = select_output_profile(&selected_options, media, 8_000);
        assert_eq!((selected.width, selected.height), (Some(1080), Some(1920)));
        assert_eq!(
            scaled_dimensions(
                media.width,
                media.height,
                selected.width.unwrap(),
                selected.height.unwrap()
            ),
            (1080, 1920)
        );
    }

    #[test]
    fn rotation_metadata_changes_display_orientation() {
        assert_eq!(display_dimensions(1920, 1080, 90.0), (1080, 1920));
        assert_eq!(display_dimensions(1920, 1080, -90.0), (1080, 1920));
        assert_eq!(display_dimensions(1920, 1080, 180.0), (1920, 1080));
    }

    #[test]
    fn gameplay_allocation_moves_bits_toward_complex_windows() {
        let motion = [
            TimedMetric {
                time: 1.0,
                value: 2.0,
            },
            TimedMetric {
                time: 5.0,
                value: 20.0,
            },
        ];
        let allocations = gameplay_allocations(8.0, &motion, None);
        assert_eq!(allocations.len(), 2);
        assert!(allocations[1].weight > allocations[0].weight);
        let mean = allocations
            .iter()
            .map(|allocation| allocation.weight * (allocation.end - allocation.start))
            .sum::<f64>()
            / 8.0;
        assert!((mean - 1.0).abs() < 0.0001);
    }

    #[test]
    fn quality_probe_has_more_influence_than_motion() {
        let motion = [
            TimedMetric {
                time: 1.0,
                value: 10.0,
            },
            TimedMetric {
                time: 5.0,
                value: 10.0,
            },
        ];
        let packets = [
            TimedMetric {
                time: 1.0,
                value: 1_000.0,
            },
            TimedMetric {
                time: 5.0,
                value: 8_000.0,
            },
        ];
        let allocations = gameplay_allocations(8.0, &motion, Some(&packets));
        assert!(allocations[1].weight > allocations[0].weight);
    }

    #[test]
    fn encoder_zone_weights_cover_every_output_frame() {
        let allocations = vec![
            SceneAllocation {
                start: 0.0,
                end: 4.0,
                weight: 0.9,
            },
            SceneAllocation {
                start: 4.0,
                end: 8.0,
                weight: 1.1,
            },
        ];
        assert_eq!(
            encoder_zones(&allocations, 30.0),
            "0,119,b=0.90/120,239,b=1.10"
        );
    }

    #[test]
    fn auto_reduces_resolution_and_fps_for_tight_budgets() {
        let media = MediaInfo {
            duration: 120.0,
            width: 3840,
            height: 2160,
            fps: 60.0,
            audio_channels: 2,
            motion: 0.85,
        };
        let selected = select_output_profile(&options("libx264"), media, 1_000);
        assert_eq!(selected.resolution_name, "540p");
        assert_eq!(selected.fps, 30.0);
    }

    #[test]
    fn auto_accounts_for_codec_efficiency() {
        let media = MediaInfo {
            duration: 30.0,
            width: 1920,
            height: 1080,
            fps: 60.0,
            audio_channels: 2,
            motion: 0.0,
        };
        let h264 = select_output_profile(&options("libx264"), media, 4_500);
        let av1 = select_output_profile(&options("libsvtav1"), media, 4_500);
        assert_eq!(h264.fps, 30.0);
        assert_eq!(av1.resolution_name, "original");
        assert_eq!(av1.fps, 60.0);
    }

    #[test]
    fn fixed_budget_reserves_room_for_audio_and_muxing() {
        let budget = fixed_size_budget(60.0, 8.0, "mp4", 2);
        let allocated_kbits =
            (budget.video_kbps + budget.audio_kbps as i64) as f64 * 60.0 + budget.reserved_kbits;
        assert!(allocated_kbits <= 8.0 * 8.0 * 1024.0);
        assert!((48..=112).contains(&budget.audio_kbps));
        assert!(budget.video_kbps > 0);
    }

    #[test]
    fn fixed_budget_does_not_allocate_audio_when_source_has_none() {
        let budget = fixed_size_budget(30.0, 8.0, "mp4", 0);
        assert_eq!(budget.audio_kbps, 0);
        assert!(budget.video_kbps > 0);
    }

    #[test]
    fn size_correction_scales_down_large_overshoots() {
        let corrected = corrected_video_bitrate(10_000, 100 * 1024 * 1024, 144 * 1024 * 1024);
        assert_eq!(corrected, 6_805);
    }

    #[test]
    fn ffmpeg_argument_updates_change_only_the_requested_value() {
        let mut args = vec![
            "-b:v".to_string(),
            "10000k".to_string(),
            "-maxrate".to_string(),
            "10000k".to_string(),
        ];
        assert!(set_ffmpeg_arg(&mut args, "-b:v", "6805k".to_string()));
        assert_eq!(args[1], "6805k");
        assert_eq!(args[3], "10000k");
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct ProgressEvent {
    pub file_index: usize,
    pub file_name: String,
    pub percent: f64,
    pub stage: String,
}
#[derive(Debug, Serialize, Clone)]
pub struct LogEvent {
    pub line: String,
}
#[derive(Debug, Serialize, Clone)]
pub struct InstallProgress {
    pub step: String,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct FileResult {
    pub file_index: usize,
    pub file_name: String,
    pub output_path: String,
    pub output_size: u64,
    pub success: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct HardwareEncoder {
    pub codec: String,
    pub codec_name: String,
    pub label: String,
    pub family: String,
    pub gpu: String,
    pub quality_mode: String,
    pub gameplay_mode: String,
}

#[derive(Debug, Serialize)]
pub struct FfmpegCheck {
    pub ffmpeg_ok: bool,
    pub ffprobe_ok: bool,
    pub ffmpeg_version: String,
    pub source: String, // "local" | "path" | "missing"
    pub path: String,
    pub build_variant: String,
    pub full_build: bool,
    pub latest_version: String,
    pub svt_version: String,
    pub update_available: bool,
}

async fn latest_gyan_version(git_build: bool) -> Option<String> {
    let url = if git_build {
        "https://www.gyan.dev/ffmpeg/builds/ffmpeg-git-full.7z.ver"
    } else {
        "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-full.7z.ver"
    };
    let script = format!(
        "$ProgressPreference='SilentlyContinue'; (Invoke-WebRequest -UseBasicParsing -Uri '{}').Content.Trim()",
        url
    );
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        hidden_cmd("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}

fn detect_svt_version(ffmpeg: &str) -> String {
    let output = hidden_std_cmd(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "info",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=64x64:r=1:d=0.1",
            "-frames:v",
            "1",
            "-an",
            "-c:v",
            "libsvtav1",
            "-f",
            "null",
            "NUL",
        ])
        .output();
    let Ok(output) = output else {
        return String::new();
    };
    let log = String::from_utf8_lossy(&output.stderr);
    let marker = "SVT-AV1 Encoder Lib v";
    log.lines()
        .find_map(|line| {
            line.find(marker)
                .map(|index| line[index + marker.len()..].trim().to_string())
        })
        .unwrap_or_default()
}

fn svt_major(version: &str) -> Option<u32> {
    version
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}
// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn check_ffmpeg(state: tauri::State<'_, AppState>) -> Result<FfmpegCheck, String> {
    let ffmpeg_info = resolve_binary_sync("ffmpeg");
    let ffprobe_info = resolve_binary_sync("ffprobe");

    // Cache resolved paths
    *state.ffmpeg.lock().unwrap() = ffmpeg_info.as_ref().map(|(p, _)| p.clone());
    *state.ffprobe.lock().unwrap() = ffprobe_info.as_ref().map(|(p, _)| p.clone());

    let source = match &ffmpeg_info {
        Some((_, s)) => s.to_string(),
        None => "missing".to_string(),
    };

    let version = if let Some((path, _)) = &ffmpeg_info {
        hidden_std_cmd(path)
            .arg("-version")
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .map(|l| l.to_string())
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let version_output = ffmpeg_info
        .as_ref()
        .and_then(|(path, _)| hidden_std_cmd(path).arg("-version").output().ok())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let lower_output = version_output.to_ascii_lowercase();
    let build_variant = if lower_output.contains("full_build") {
        "full"
    } else if lower_output.contains("essentials_build") {
        "essentials"
    } else {
        "custom"
    }
    .to_string();
    let full_build = build_variant == "full";
    let installed_id = version
        .strip_prefix("ffmpeg version ")
        .unwrap_or(&version)
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let is_git = installed_id.contains("-git-");
    let latest_version = if ffmpeg_info.is_some() {
        latest_gyan_version(is_git).await.unwrap_or_default()
    } else {
        String::new()
    };

    let path = ffmpeg_info
        .as_ref()
        .map(|(path, _)| path.clone())
        .unwrap_or_default();
    let svt_version = if path.is_empty() {
        String::new()
    } else {
        detect_svt_version(&path)
    };
    let old_svt = svt_major(&svt_version).is_some_and(|major| major < 4);
    let online_update = !latest_version.is_empty() && !installed_id.starts_with(&latest_version);
    let update_available = online_update || old_svt;

    Ok(FfmpegCheck {
        ffmpeg_ok: ffmpeg_info.is_some(),
        ffprobe_ok: ffprobe_info.is_some(),
        ffmpeg_version: version,
        source,
        path,
        build_variant,
        full_build,
        latest_version,
        svt_version,
        update_available,
    })
}

fn gpu_for_family(gpus: &[String], family: &str) -> String {
    let matches: Vec<String> = gpus
        .iter()
        .filter(|name| {
            let n = name.to_ascii_lowercase();
            match family {
                "nvenc" => {
                    n.contains("nvidia")
                        || n.contains("geforce")
                        || n.contains("quadro")
                        || n.contains("tesla")
                }
                "amf" => n.contains("amd") || n.contains("radeon"),
                "qsv" => n.contains("intel"),
                _ => false,
            }
        })
        .cloned()
        .collect();
    if family == "software" {
        "CPU".into()
    } else if matches.is_empty() {
        "Compatible GPU".into()
    } else {
        matches
            .iter()
            .map(|name| compact_gpu_name(name))
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

fn compact_gpu_name(name: &str) -> String {
    let ignored = [
        "amd", "radeon", "nvidia", "geforce", "intel", "intel(r)", "graphics", "(tm)", "arc",
        "arc(tm)",
    ];
    let compact = name
        .split_whitespace()
        .filter(|part| !ignored.contains(&part.to_ascii_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    if compact.is_empty() {
        name.to_string()
    } else {
        compact
    }
}

async fn installed_gpu_names() -> Vec<String> {
    #[cfg(windows)]
    {
        let output = hidden_cmd("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue).Name",
            ])
            .output()
            .await;
        output
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    Vec::new()
}

async fn test_encoder_configuration(
    state: &AppState,
    ffmpeg: &str,
    codec: &str,
    pixel_format: &str,
    extra_args: &[&str],
) -> bool {
    let mut command = hidden_cmd(ffmpeg);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=black:s=640x360:r=30",
        "-frames:v",
        "1",
        "-an",
        "-c:v",
        codec,
        "-pix_fmt",
        pixel_format,
    ]);
    command.args(extra_args);
    command
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let Ok(mut child) = command.spawn() else {
        return false;
    };
    #[cfg(windows)]
    state.process_job.assign(&child);
    match tokio::time::timeout(std::time::Duration::from_secs(15), child.wait()).await {
        Ok(Ok(status)) => status.success(),
        _ => {
            let _ = child.start_kill();
            false
        }
    }
}

#[tauri::command]
async fn detect_hardware_encoders(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<HardwareEncoder>, String> {
    let ffmpeg = state
        .ffmpeg
        .lock()
        .unwrap()
        .clone()
        .ok_or("FFmpeg not available")?;
    let gpus = installed_gpu_names().await;
    let candidates = [
        ("libsvtav1", "svt-av1", "SVT-AV1", "software"),
        ("h264_nvenc", "h264-nvenc", "H.264 (NVENC)", "nvenc"),
        ("hevc_nvenc", "h265-nvenc", "H.265 (NVENC)", "nvenc"),
        ("av1_nvenc", "av1-nvenc", "AV1 (NVENC)", "nvenc"),
        ("h264_amf", "h264-amf", "H.264 (AMF)", "amf"),
        ("hevc_amf", "h265-amf", "H.265 (AMF)", "amf"),
        ("av1_amf", "av1-amf", "AV1 (AMF)", "amf"),
        ("h264_qsv", "h264-qsv", "H.264 (QSV)", "qsv"),
        ("hevc_qsv", "h265-qsv", "H.265 (QSV)", "qsv"),
        ("av1_qsv", "av1-qsv", "AV1 (QSV)", "qsv"),
    ];
    let mut supported = Vec::new();
    for (codec, codec_name, label, family) in candidates {
        let pixel_format = if codec == "libsvtav1" {
            "yuv420p10le"
        } else {
            "nv12"
        };
        let base_ok = test_encoder_configuration(&state, &ffmpeg, codec, pixel_format, &[]).await;
        let quality_args: &[&str] = match family {
            "nvenc" => &["-rc", "vbr", "-cq", "24", "-b:v", "0"],
            "amf" => &["-rc", "qvbr", "-qvbr_quality_level", "24"],
            "qsv" => &["-global_quality", "24"],
            _ => &["-crf", "24"],
        };
        let mut quality_ok = base_ok
            && test_encoder_configuration(&state, &ffmpeg, codec, pixel_format, quality_args).await;
        let mut quality_mode = match family {
            "nvenc" => "nvenc_cqvbr",
            "amf" => "amf_qvbr",
            "qsv" => "qsv_icq",
            _ => "crf",
        };
        // Some AMF drivers advertise QVBR through FFmpeg but reject it during
        // initialization. Keep the encoder available with its hardware-quality
        // control when that happens; the selected mode is carried into encoding.
        if base_ok && family == "amf" && !quality_ok {
            let fallback = &["-rc", "cqp", "-qp_i", "24", "-qp_p", "24"];
            quality_ok =
                test_encoder_configuration(&state, &ffmpeg, codec, pixel_format, fallback).await;
            quality_mode = "amf_quality_fallback";
        }
        let gameplay_args: &[&str] = match family {
            "nvenc" => &[
                "-b:v",
                "500k",
                "-maxrate",
                "625k",
                "-bufsize",
                "1000k",
                "-rc",
                "vbr",
                "-multipass",
                "fullres",
                "-spatial-aq",
                "1",
                "-temporal-aq",
                "1",
                "-aq-strength",
                "8",
                "-rc-lookahead",
                "32",
            ],
            "amf" => &[
                "-b:v",
                "500k",
                "-maxrate",
                "625k",
                "-bufsize",
                "1000k",
                "-rc",
                "vbr_peak",
                "-preanalysis",
                "true",
                "-pa_high_motion_quality_boost_mode",
                "auto",
                "-pa_taq_mode",
                "2",
            ],
            "qsv" => &[
                "-b:v",
                "500k",
                "-maxrate",
                "625k",
                "-bufsize",
                "1000k",
                "-extbrc",
                "1",
                "-look_ahead_depth",
                "40",
                "-adaptive_i",
                "1",
                "-adaptive_b",
                "1",
            ],
            _ => &[],
        };
        let gameplay_mode = if family == "software" {
            "native"
        } else if base_ok
            && test_encoder_configuration(&state, &ffmpeg, codec, pixel_format, gameplay_args).await
        {
            match family {
                "nvenc" => "nvenc_aq",
                "amf" => "amf_preanalysis",
                "qsv" => "qsv_extbrc",
                _ => "internal",
            }
        } else {
            "internal"
        };
        let ok = base_ok && quality_ok;
        if ok {
            supported.push(HardwareEncoder {
                codec: codec.into(),
                codec_name: codec_name.into(),
                label: label.into(),
                family: family.into(),
                gpu: gpu_for_family(&gpus, family),
                quality_mode: quality_mode.into(),
                gameplay_mode: gameplay_mode.into(),
            });
        }
    }
    Ok(supported)
}

#[tauri::command]
async fn cancel_file(state: tauri::State<'_, AppState>, file_index: usize) -> Result<(), String> {
    state.cancelled_files.lock().unwrap().insert(file_index);
    let pid = state.active_pids.lock().unwrap().get(&file_index).copied();
    if let Some(pid) = pid {
        terminate_pid(pid).await;
    }
    Ok(())
}

#[tauri::command]
async fn cancel_all_compressions(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel_all.store(true, Ordering::SeqCst);
    let pids: Vec<u32> = state
        .active_pids
        .lock()
        .unwrap()
        .values()
        .copied()
        .collect();
    for pid in pids {
        terminate_pid(pid).await;
    }
    Ok(())
}

#[tauri::command]
async fn cancel_all_and_close(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    state.cancel_all.store(true, Ordering::SeqCst);
    state.allow_close.store(true, Ordering::SeqCst);
    let pids: Vec<u32> = state
        .active_pids
        .lock()
        .unwrap()
        .values()
        .copied()
        .collect();
    for pid in pids {
        terminate_pid(pid).await;
    }
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn select_files(app: AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = oneshot::channel::<Vec<String>>();
    app.dialog()
        .file()
        .add_filter(
            "Video Files",
            &[
                "mp4", "mkv", "avi", "mov", "webm", "flv", "m4v", "wmv", "ts", "m2ts",
            ],
        )
        .pick_files(move |r| {
            let _ = tx.send(
                r.unwrap_or_default()
                    .into_iter()
                    .map(|p| p.to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            );
        });
    rx.await.map_err(|_| "Dialog closed".to_string())
}

#[tauri::command]
async fn select_output_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = oneshot::channel::<Option<String>>();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(
            folder
                .map(|path| path.to_string())
                .filter(|path| !path.is_empty()),
        );
    });
    rx.await.map_err(|_| "Dialog closed".to_string())
}

#[tauri::command]
async fn install_ffmpeg(state: tauri::State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    // PowerShell handles download → extract → copy, emitting STATUS:/ERROR: lines
    let ps = r#"
$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function ws($m) { Write-Output "STATUS:$m"; [Console]::Out.Flush() }
function we($m) { Write-Output "ERROR:$m";  [Console]::Out.Flush() }

$url     = 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-full.7z'
$hashUrl = 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-full.7z.sha256'
$tmp     = [System.IO.Path]::GetTempPath()
$archive = Join-Path $tmp 'effigy_ffmpeg_full.7z'
$hash    = Join-Path $tmp 'effigy_ffmpeg_full.sha256'
$extract = Join-Path $tmp 'effigy_ffmpeg_extract'
$dest    = Join-Path ([System.Environment]::GetFolderPath('LocalApplicationData')) 'Programs\FFmpeg\bin'

try {
    if (-not (Test-Path $dest)) { New-Item -ItemType Directory -Path $dest -Force | Out-Null }

    ws 'Downloading FFmpeg Full...'
    Invoke-WebRequest -Uri $url -OutFile $archive -UseBasicParsing
    Invoke-WebRequest -Uri $hashUrl -OutFile $hash -UseBasicParsing

    ws 'Verifying download...'
    $expected = (Get-Content $hash -Raw).Trim().Split()[0].ToLowerInvariant()
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw 'FFmpeg archive checksum verification failed' }

    ws 'Extracting archive...'
    if (Test-Path $extract) { Remove-Item $extract -Recurse -Force }
    New-Item -ItemType Directory -Path $extract -Force | Out-Null
    & tar.exe -xf $archive -C $extract
    if ($LASTEXITCODE -ne 0) { throw "FFmpeg archive extraction failed with code $LASTEXITCODE" }

    ws 'Copying binaries...'
    $ff = Get-ChildItem -Path $extract -Recurse -Filter 'ffmpeg.exe'  | Select-Object -First 1
    $fp = Get-ChildItem -Path $extract -Recurse -Filter 'ffprobe.exe' | Select-Object -First 1
    if (-not $ff) { throw 'ffmpeg.exe not found in archive' }
    if (-not $fp) { throw 'ffprobe.exe not found in archive' }
    Copy-Item $ff.FullName "$dest\ffmpeg.exe"  -Force
    Copy-Item $fp.FullName "$dest\ffprobe.exe" -Force

    ws 'Configuring user PATH...'
    $userPath = [System.Environment]::GetEnvironmentVariable('Path', 'User')
    $normalizedDest = $dest.TrimEnd('\')
    $alreadyPresent = @($userPath -split ';') | Where-Object {
        $_.Trim().TrimEnd('\') -ieq $normalizedDest
    }
    if (-not $alreadyPresent) {
        $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $dest } else { "$userPath;$dest" }
        [System.Environment]::SetEnvironmentVariable('Path', $newPath, 'User')

        Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class EnvironmentBroadcast {
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr SendMessageTimeout(
        IntPtr hWnd, uint message, IntPtr wParam, string lParam,
        uint flags, uint timeout, out IntPtr result);
}
"@
        $broadcastResult = [IntPtr]::Zero
        [EnvironmentBroadcast]::SendMessageTimeout(
            [IntPtr]0xffff, 0x001A, [IntPtr]::Zero, 'Environment',
            0x0002, 5000, [ref]$broadcastResult) | Out-Null
        ws 'Added FFmpeg to the user PATH.'
    } else {
        ws 'FFmpeg is already in the user PATH.'
    }

    ws 'Done!'
} catch {
    we "$_"
} finally {
    Remove-Item $archive -Force          -ErrorAction SilentlyContinue
    Remove-Item $hash    -Force          -ErrorAction SilentlyContinue
    Remove-Item $extract -Recurse -Force -ErrorAction SilentlyContinue
}
"#;

    let mut child = hidden_cmd("powershell")
        .args(["-NoProfile", "-Command", ps])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start PowerShell: {e}"))?;
    #[cfg(windows)]
    state.process_job.assign(&child);

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(step) = line.strip_prefix("STATUS:") {
                let done = step == "Done!";
                let _ = app.emit(
                    "ffmpeg-install",
                    InstallProgress {
                        step: step.to_string(),
                        done,
                        error: None,
                    },
                );
            } else if let Some(err) = line.strip_prefix("ERROR:") {
                let _ = app.emit(
                    "ffmpeg-install",
                    InstallProgress {
                        step: String::new(),
                        done: false,
                        error: Some(err.to_string()),
                    },
                );
            }
        }
    }

    child.wait().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn compress_files(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
    files: Vec<String>,
    options: CompressOptions,
) -> Result<Vec<FileResult>, String> {
    let ffmpeg = state
        .ffmpeg
        .lock()
        .unwrap()
        .clone()
        .ok_or("FFmpeg not found — please install it first.")?;
    let ffprobe = state
        .ffprobe
        .lock()
        .unwrap()
        .clone()
        .ok_or("FFprobe not found — please install it first.")?;

    if state.running.swap(true, Ordering::SeqCst) {
        return Err("A compression queue is already running".into());
    }
    state.cancel_all.store(false, Ordering::SeqCst);
    state.cancelled_files.lock().unwrap().clear();

    if options.container != "mp4" && options.container != "webm" {
        state.running.store(false, Ordering::SeqCst);
        return Err("Unsupported output container".into());
    }
    let is_av1 = options.codec.starts_with("av1_") || options.codec == "libsvtav1";
    if options.container == "webm" && !is_av1 {
        state.running.store(false, Ordering::SeqCst);
        return Err("WebM is available for AV1 encoders; choose MP4 for H.264 or H.265".into());
    }

    let mut results: Vec<FileResult> = Vec::new();

    for (idx, input) in files.iter().enumerate() {
        let path = std::path::Path::new(input);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();
        let parent = path
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .to_string();
        let quality_name = if options.algorithm == "dynamic" {
            format!("crf{}", options.crf)
        } else {
            options.size_name.clone()
        };
        let output_parent = options
            .output_folder
            .as_deref()
            .filter(|folder| !folder.trim().is_empty())
            .unwrap_or(&parent);
        if let Err(error) = std::fs::create_dir_all(output_parent) {
            results.push(FileResult {
                file_index: idx,
                file_name: stem,
                output_path: String::new(),
                output_size: 0,
                success: false,
                cancelled: false,
                error: Some(format!("Unable to create output folder: {error}")),
            });
            continue;
        }
        macro_rules! emit_p {
            ($pct:expr, $stage:expr) => {
                let _ = app.emit(
                    "progress",
                    ProgressEvent {
                        file_index: idx,
                        file_name: stem.clone(),
                        percent: $pct,
                        stage: $stage.to_string(),
                    },
                );
            };
        }
        macro_rules! emit_l {
            ($line:expr) => {
                let _ = app.emit(
                    "ffmpeg-log",
                    LogEvent {
                        line: $line.to_string(),
                    },
                );
            };
        }

        if is_cancelled(&state, idx) {
            emit_p!(-2.0, "Cancelled");
            results.push(FileResult {
                file_index: idx,
                file_name: stem,
                output_path: String::new(),
                output_size: 0,
                success: false,
                cancelled: true,
                error: None,
            });
            continue;
        }

        emit_p!(0.0, "Analyzing…");
        emit_l!(format!("── [{}/{}] {} ──", idx + 1, files.len(), stem));

        let mut media = match probe_media(&state, &ffprobe, input).await {
            Ok(media) => media,
            Err(e) => {
                emit_p!(-1.0, format!("Error: {e}"));
                emit_l!(format!("[ERROR] {e}"));
                results.push(FileResult {
                    file_index: idx,
                    file_name: stem,
                    output_path: String::new(),
                    output_size: 0,
                    success: false,
                    cancelled: false,
                    error: Some(e),
                });
                continue;
            }
        };
        let duration = media.duration;
        let motion_samples = if options.algorithm == "fixed" {
            emit_p!(0.0, "Analyzing gameplay motion…");
            let samples = probe_motion_samples(&state, &ffmpeg, input, idx).await;
            media.motion = mean_motion_complexity(&samples);
            emit_l!(format!(
                "[INFO] Source motion complexity: {:.0}%",
                media.motion * 100.0
            ));
            samples
        } else {
            Vec::new()
        };

        let budget = if options.algorithm == "fixed" {
            fixed_size_budget(
                duration,
                options.target_size,
                &options.container,
                media.audio_channels,
            )
        } else {
            EncodingBudget {
                video_kbps: 0,
                audio_kbps: adaptive_audio_kbps(&options.container, media.audio_channels, None),
                reserved_kbits: 0.0,
            }
        };
        let mut bitrate = budget.video_kbps;
        let audio_kbps = budget.audio_kbps;
        if options.algorithm == "fixed" && options.codec == "av1_amf" {
            // AMF AV1 peak-VBR can finish slightly above its requested average.
            // Reserve an extra 3% so strict upload limits are not exceeded.
            bitrate = (bitrate as f64 * 0.97).round() as i64;
            emit_l!("[INFO] AV1 AMF: reserving 3% bitrate headroom for size accuracy");
        }
        if options.algorithm == "fixed" && bitrate <= 0 {
            let msg = format!("Bitrate {bitrate} kbps — clip too short for target size?");
            emit_p!(-1.0, format!("Error: {msg}"));
            emit_l!(format!("[ERROR] {msg}"));
            results.push(FileResult {
                file_index: idx,
                file_name: stem,
                output_path: String::new(),
                output_size: 0,
                success: false,
                cancelled: false,
                error: Some(msg),
            });
            continue;
        }

        let output_profile = select_output_profile(&options, media, bitrate);
        if options.res_mode == "auto" || options.fps_name == "auto" {
            emit_l!(format!(
                "[INFO] Auto output: {} at {} (source: {}x{} at {:.2} fps)",
                output_profile.resolution_name,
                output_profile.fps_name,
                media.width,
                media.height,
                media.fps
            ));
        }
        let supports_custom_scene_allocation =
            matches!(options.codec.as_str(), "libx264" | "libx265" | "libsvtav1");
        let probe_packets = if options.algorithm == "fixed"
            && options.allocation_mode == "probe"
            && supports_custom_scene_allocation
        {
            emit_p!(0.0, "Running quality probe…");
            emit_l!("[INFO] Gameplay allocation: constant-quality probe analysis");
            match probe_quality_packets(&state, &ffmpeg, &ffprobe, input, idx).await {
                Ok(packets) if !packets.is_empty() => Some(packets),
                Ok(_) => {
                    emit_l!("[WARN] Quality probe returned no packets; using lightweight motion allocation");
                    None
                }
                Err(error) => {
                    emit_l!(format!(
                        "[WARN] {error}; using lightweight motion allocation"
                    ));
                    None
                }
            }
        } else {
            if options.algorithm == "fixed"
                && options.allocation_mode == "probe"
                && !supports_custom_scene_allocation
            {
                emit_l!(
                    "[INFO] GPU encoder: custom probe zones are unavailable; using its tested hardware lookahead/AQ path"
                );
            }
            None
        };
        let scene_allocations =
            if options.algorithm == "fixed" {
                let allocations =
                    gameplay_allocations(duration, &motion_samples, probe_packets.as_deref());
                let minimum = allocations
                    .iter()
                    .map(|allocation| allocation.weight)
                    .fold(f64::INFINITY, f64::min);
                let maximum = allocations
                    .iter()
                    .map(|allocation| allocation.weight)
                    .fold(f64::NEG_INFINITY, f64::max);
                emit_l!(format!(
                "[INFO] Gameplay allocation: {} windows, {:.2}x–{:.2}x normalized scene weights",
                allocations.len(), minimum, maximum
            ));
                Some(allocations)
            } else {
                None
            };
        let pixel_format = if options.codec == "libsvtav1" {
            "yuv420p10le"
        } else {
            "yuv420p"
        };
        // Lanczos downscaling, even dimensions, and CFR output maximize compatibility.
        let vf = if let (Some(width), Some(height)) = (output_profile.width, output_profile.height)
        {
            format!(
                "fps={fps},scale={width}:{height}:flags=lanczos:force_original_aspect_ratio=decrease:force_divisible_by=2,format={pixel_format}",
                fps = output_profile.fps,
            )
        } else {
            format!(
                "fps={},scale=trunc(iw/2)*2:trunc(ih/2)*2:flags=lanczos,format={pixel_format}",
                output_profile.fps,
            )
        };
        let out_name = format!(
            "{}-{}-{}-{}-{}.{}",
            stem,
            options.codec_name,
            output_profile.resolution_name,
            output_profile.fps_name,
            quality_name,
            options.container
        );
        let out_path = std::path::Path::new(output_parent)
            .join(&out_name)
            .to_string_lossy()
            .to_string();
        // ABR controls the global size while VBV permits bounded scene peaks.
        // The final hard-limit verification below remains authoritative.
        let peak_bitrate = (bitrate * 5) / 4;
        let bufsize = bitrate * 2;

        let profile = if options.codec == "libx264" || options.codec.starts_with("h264_") {
            "high"
        } else {
            "main"
        };

        if options.algorithm == "fixed" {
            emit_l!(format!(
                "[INFO] Hard-size budget: video {bitrate} kbps | adaptive audio {audio_kbps} kbps | reserved {:.1} KiB",
                budget.reserved_kbits / 8.0
            ));
            emit_p!(0.0, format!("Compressing… ({bitrate} kbps)"));
        } else {
            emit_l!(format!(
                "[INFO] Duration: {duration:.2}s | CRF: {} | adaptive audio: {audio_kbps} kbps",
                options.crf
            ));
            emit_p!(0.0, format!("Compressing… (CRF {})", options.crf));
        }

        // Build common video arguments, then add rate-control flags per encoder API.
        let mut ff_args: Vec<String> = vec![
            "-hide_banner".into(),
            "-i".into(),
            input.clone(),
            // Keep stream selection identical across both software passes.
            // Timestamp-discontinuous captures can otherwise produce a
            // shorter x264 MB-tree stats file when pass 1 disables audio.
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "0:a:0?".into(),
            "-map_metadata".into(),
            "0".into(),
            "-map_chapters".into(),
            "0".into(),
            "-vf".into(),
            vf.clone(),
            "-c:v".into(),
            options.codec.clone(),
            "-pix_fmt".into(),
            pixel_format.into(),
            "-profile:v".into(),
            profile.into(),
            "-colorspace".into(),
            "bt709".into(),
            "-color_primaries".into(),
            "bt709".into(),
            "-color_trc".into(),
            "bt709".into(),
            "-g".into(),
            ((output_profile.fps * 10.0).round() as u32)
                .clamp(120, 600)
                .to_string(),
        ];

        let family = if options.codec == "libsvtav1" {
            "svt"
        } else if options.codec.ends_with("_nvenc") {
            "nvenc"
        } else if options.codec.ends_with("_amf") {
            "amf"
        } else if options.codec.ends_with("_qsv") {
            "qsv"
        } else {
            "software"
        };
        let use_two_pass = options.algorithm == "fixed"
            && matches!(options.codec.as_str(), "libx264" | "libx265" | "libsvtav1");
        let zones = scene_allocations
            .as_deref()
            .filter(|_| matches!(options.codec.as_str(), "libx264" | "libx265"))
            .map(|allocations| encoder_zones(allocations, output_profile.fps));
        let mapped_preset: String = match family {
            "nvenc" => match options.preset.as_str() {
                "slow" => "p5",
                "slower" => "p6",
                "veryslow" => "p7",
                _ => "p4",
            }
            .into(),
            "amf" => {
                if options.preset == "medium" {
                    "balanced"
                } else {
                    "quality"
                }
            }
            .into(),
            "svt" => options.svt_preset.min(13).to_string(),
            _ => options.preset.clone(),
        };
        if family == "amf" {
            ff_args.extend(["-quality".into(), mapped_preset]);
        } else {
            ff_args.extend(["-preset".into(), mapped_preset]);
        }

        if options.algorithm == "fixed" {
            ff_args.extend(["-b:v".into(), format!("{bitrate}k")]);
            // SVT-AV1 explicitly rejects maxrate in ABR mode. x264/x265 and
            // hardware VBR encoders accept these VBV constraints.
            if family != "svt" {
                ff_args.extend([
                    "-maxrate".into(),
                    format!("{peak_bitrate}k"),
                    "-bufsize".into(),
                    format!("{bufsize}k"),
                ]);
            }
            if use_two_pass {
                if family == "svt" {
                    emit_l!(format!(
                        "[INFO] Rate control: SVT-AV1 two-pass ABR ({bitrate}k average); final hard-size verification enabled"
                    ));
                } else {
                    emit_l!(format!(
                        "[INFO] Rate control: two-pass ABR + VBV ({bitrate}k average, {peak_bitrate}k peak, {bufsize}k buffer)"
                    ));
                }
            } else {
                match family {
                    "nvenc" => {
                        ff_args.extend(["-rc".into(), "vbr".into()]);
                        if options.gpu_gameplay_mode == "nvenc_aq" {
                            ff_args.extend([
                                "-multipass".into(),
                                "fullres".into(),
                                "-spatial-aq".into(),
                                "1".into(),
                                "-temporal-aq".into(),
                                "1".into(),
                                "-aq-strength".into(),
                                "8".into(),
                                "-rc-lookahead".into(),
                                "32".into(),
                            ]);
                            emit_l!("[INFO] GPU gameplay allocation: NVENC lookahead + spatial/temporal AQ");
                        }
                        emit_l!(
                            "[INFO] Rate control: NVENC full-resolution multipass constrained VBR"
                        );
                    }
                    "amf" => {
                        ff_args.extend(["-rc".into(), "vbr_peak".into()]);
                        if options.gpu_gameplay_mode == "amf_preanalysis" {
                            ff_args.extend([
                                "-preanalysis".into(),
                                "true".into(),
                                "-pa_high_motion_quality_boost_mode".into(),
                                "auto".into(),
                                "-pa_taq_mode".into(),
                                "2".into(),
                            ]);
                            emit_l!("[INFO] GPU gameplay allocation: AMF pre-analysis + temporal AQ + high-motion boost");
                        }
                        emit_l!("[INFO] Rate control: AMF peak-constrained VBR");
                    }
                    "qsv" => {
                        if options.gpu_gameplay_mode == "qsv_extbrc" {
                            ff_args.extend([
                                "-extbrc".into(),
                                "1".into(),
                                "-look_ahead_depth".into(),
                                "40".into(),
                                "-adaptive_i".into(),
                                "1".into(),
                                "-adaptive_b".into(),
                                "1".into(),
                            ]);
                            emit_l!("[INFO] GPU gameplay allocation: QSV extended bitrate control + lookahead");
                        }
                        emit_l!("[INFO] Rate control: QSV bitrate-constrained VBR");
                    }
                    _ => {}
                }
            }
        } else {
            let q = options.crf.min(51).to_string();
            match family {
                "nvenc" => ff_args.extend([
                    "-rc".into(),
                    "vbr".into(),
                    "-cq".into(),
                    q,
                    "-b:v".into(),
                    "0".into(),
                ]),
                "amf" if options.quality_mode == "amf_qvbr" => {
                    ff_args.extend(["-rc".into(), "qvbr".into(), "-qvbr_quality_level".into(), q])
                }
                "amf" => {
                    ff_args.extend([
                        "-rc".into(),
                        "cqp".into(),
                        "-qp_i".into(),
                        q.clone(),
                        "-qp_p".into(),
                        q.clone(),
                    ]);
                    // HEVC AMF exposes I/P quantizers but no B-frame quantizer.
                    if options.codec != "hevc_amf" {
                        ff_args.extend(["-qp_b".into(), q]);
                    }
                }
                "qsv" => ff_args.extend(["-global_quality".into(), q]),
                _ => ff_args.extend(["-crf".into(), q]),
            }
        }

        if family == "svt" {
            let allocation_spread = scene_allocations
                .as_deref()
                .map(|allocations| {
                    let minimum = allocations
                        .iter()
                        .map(|allocation| allocation.weight)
                        .fold(1.0_f64, f64::min);
                    let maximum = allocations
                        .iter()
                        .map(|allocation| allocation.weight)
                        .fold(1.0_f64, f64::max);
                    maximum - minimum
                })
                .unwrap_or(0.0);
            let variance_strength =
                if options.allocation_mode == "probe" && allocation_spread >= 0.20 {
                    3
                } else {
                    2
                };
            let mut params = format!(
                "tune=0:enable-variance-boost=1:variance-boost-strength={variance_strength}:aq-mode=2"
            );
            if options.algorithm == "fixed" {
                params.insert_str(0, "rc=1:");
                emit_l!(format!(
                    "[INFO] SVT-AV1 gameplay allocation: scene-aware VBR + AQ2 + variance boost {variance_strength}"
                ));
            }
            ff_args.extend(["-svtav1-params".into(), params]);
            emit_l!("[INFO] SVT-AV1: 10-bit VQ tune with motion-calibrated quality redistribution");
        }

        // Software encoder-specific quality tuning. Keyframe placement is left
        // to each encoder so scene changes can receive I-frames naturally.
        if options.codec == "libx264" {
            // aq-mode=3       → auto-variance adaptive quantization (best for gaming)
            // aq-strength=0.8 → moderate AQ — keeps detail without overspending bits
            // psy-rd=1.0,0.15 → psychovisual RD + trellis; keeps perceived sharpness
            let mut params = String::from("aq-mode=3:aq-strength=0.8:psy-rd=1.0,0.15");
            if let Some(zones) = &zones {
                params.push_str(":zones=");
                params.push_str(zones);
                emit_l!("[INFO] x264 gameplay allocation: normalized bitrate zones enabled");
            }
            ff_args.extend(["-x264-params".into(), params]);
        } else if options.codec == "libx265" {
            // x265 equivalent: preserve AQ and use its automatic scene cuts.
            let mut params = String::from("aq-mode=3");
            if let Some(zones) = &zones {
                params.push_str(":zones=");
                params.push_str(zones);
                emit_l!("[INFO] x265 gameplay allocation: normalized bitrate zones enabled");
            }
            ff_args.extend(["-x265-params".into(), params]);
        }

        let passlog = if use_two_pass {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            Some(
                std::env::temp_dir()
                    .join(format!("effigy-pass-{}-{idx}-{nonce}", std::process::id())),
            )
        } else {
            None
        };

        if let Some(prefix) = &passlog {
            emit_l!("[INFO] Two-pass fixed-size encode: analysis pass 1/2");
            emit_p!(0.0, format!("Pass 1/2 • {bitrate} kbps"));
            let mut first_args = ff_args.clone();
            add_two_pass_args(&mut first_args, &options.codec, 1, prefix);
            first_args.extend([
                // Demux the same audio stream as pass 2 without encoding it.
                "-c:a".into(),
                "copy".into(),
                "-progress".into(),
                "pipe:1".into(),
                "-nostats".into(),
                "-fps_mode".into(),
                "cfr".into(),
                "-y".into(),
                "-f".into(),
                "null".into(),
                if cfg!(windows) {
                    "NUL".into()
                } else {
                    "/dev/null".into()
                },
            ]);
            let stage = format!("Pass 1/2 • {bitrate} kbps •");
            let first_status = run_ffmpeg_process(
                &state,
                &app,
                FfmpegRun {
                    ffmpeg: &ffmpeg,
                    file_index: idx,
                    file_name: &stem,
                    args: &first_args,
                    duration,
                    progress_start: 0.0,
                    progress_span: 45.0,
                    stage: &stage,
                },
            )
            .await;

            if is_cancelled(&state, idx) {
                cleanup_passlogs(prefix);
                emit_p!(-2.0, "Cancelled");
                results.push(FileResult {
                    file_index: idx,
                    file_name: stem,
                    output_path: String::new(),
                    output_size: 0,
                    success: false,
                    cancelled: true,
                    error: None,
                });
                continue;
            }
            match first_status {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    cleanup_passlogs(prefix);
                    let msg = format!(
                        "FFmpeg analysis pass exited with code {}",
                        status.code().unwrap_or(-1)
                    );
                    emit_p!(-1.0, format!("Error: {msg}"));
                    emit_l!(format!("[ERROR] {msg}"));
                    results.push(FileResult {
                        file_index: idx,
                        file_name: stem,
                        output_path: String::new(),
                        output_size: 0,
                        success: false,
                        cancelled: false,
                        error: Some(msg),
                    });
                    continue;
                }
                Err(msg) => {
                    cleanup_passlogs(prefix);
                    emit_p!(-1.0, format!("Error: {msg}"));
                    emit_l!(format!("[ERROR] {msg}"));
                    results.push(FileResult {
                        file_index: idx,
                        file_name: stem,
                        output_path: String::new(),
                        output_size: 0,
                        success: false,
                        cancelled: false,
                        error: Some(msg),
                    });
                    continue;
                }
            }
            add_two_pass_args(&mut ff_args, &options.codec, 2, prefix);
            emit_l!("[INFO] Two-pass fixed-size encode: output pass 2/2");
        }

        if media.audio_channels > 0 {
            let encoded_audio_kbps = audio_kbps;
            let audio_codec = if options.container == "webm" {
                "libopus"
            } else {
                "aac"
            };
            ff_args.extend([
                "-c:a".into(),
                audio_codec.into(),
                "-b:a".into(),
                format!("{encoded_audio_kbps}k"),
                "-ar".into(),
                "48000".into(),
                "-ac".into(),
                media.audio_channels.min(2).to_string(),
            ]);
            if options.container == "mp4" {
                emit_l!(format!("[INFO] Audio: AAC {encoded_audio_kbps} kbps"));
            } else {
                emit_l!(format!("[INFO] Audio: Opus {encoded_audio_kbps} kbps"));
            }
        }
        if options.container == "mp4" {
            ff_args.extend(["-movflags".into(), "+faststart".into()]);
        }
        ff_args.extend([
            "-progress".into(),
            "pipe:1".into(),
            "-nostats".into(),
            "-fps_mode".into(),
            "cfr".into(),
            "-y".into(),
            out_path.clone(),
        ]);

        let (progress_start, progress_span, stage) = if use_two_pass {
            (45.0, 54.0, format!("Pass 2/2 • {bitrate} kbps •"))
        } else if options.algorithm == "fixed" {
            (0.0, 99.0, format!("Compressing… • {bitrate} kbps •"))
        } else {
            (0.0, 99.0, format!("Compressing… • CRF {} •", options.crf))
        };
        let mut status = match run_ffmpeg_process(
            &state,
            &app,
            FfmpegRun {
                ffmpeg: &ffmpeg,
                file_index: idx,
                file_name: &stem,
                args: &ff_args,
                duration,
                progress_start,
                progress_span,
                stage: &stage,
            },
        )
        .await
        {
            Ok(status) => status,
            Err(msg) => {
                if let Some(prefix) = &passlog {
                    cleanup_passlogs(prefix);
                }
                emit_p!(-1.0, format!("Error: {msg}"));
                emit_l!(format!("[ERROR] {msg}"));
                results.push(FileResult {
                    file_index: idx,
                    file_name: stem,
                    output_path: String::new(),
                    output_size: 0,
                    success: false,
                    cancelled: false,
                    error: Some(msg),
                });
                continue;
            }
        };
        let hard_limit_bytes = (options.target_size * 1024.0 * 1024.0).floor() as u64;
        let mut retry_error: Option<String> = None;
        if options.algorithm == "fixed" && status.success() {
            // Rate-control accuracy varies by encoder, driver, and content.
            // Verify the actual container size and automatically correct any
            // encoder that did not honor its requested average bitrate. For
            // software two-pass encoders, the analysis stats remain valid for
            // a bitrate-corrected second pass.
            for retry in 1..=2 {
                let actual_bytes = std::fs::metadata(&out_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                if actual_bytes <= hard_limit_bytes || is_cancelled(&state, idx) {
                    break;
                }
                let corrected_bitrate =
                    corrected_video_bitrate(bitrate, hard_limit_bytes, actual_bytes);
                emit_l!(format!(
                    "[WARN] Output was {:.2} MB for a {:.2} MB limit; retrying at {corrected_bitrate} kbps ({retry}/2)",
                    actual_bytes as f64 / 1024.0 / 1024.0,
                    options.target_size
                ));
                emit_p!(
                    0.0,
                    format!("Correcting sizeâ€¦ ({corrected_bitrate} kbps)")
                );
                bitrate = corrected_bitrate;
                let _ = set_ffmpeg_arg(&mut ff_args, "-b:v", format!("{bitrate}k"));
                let _ = set_ffmpeg_arg(&mut ff_args, "-maxrate", format!("{}k", (bitrate * 5) / 4));
                let _ = set_ffmpeg_arg(&mut ff_args, "-bufsize", format!("{}k", bitrate * 2));
                let _ = std::fs::remove_file(&out_path);
                let retry_stage = format!("Size correction {retry}/2 â€¢ {bitrate} kbps â€¢");
                status = match run_ffmpeg_process(
                    &state,
                    &app,
                    FfmpegRun {
                        ffmpeg: &ffmpeg,
                        file_index: idx,
                        file_name: &stem,
                        args: &ff_args,
                        duration,
                        progress_start: 0.0,
                        progress_span: 99.0,
                        stage: &retry_stage,
                    },
                )
                .await
                {
                    Ok(status) => status,
                    Err(error) => {
                        retry_error = Some(error);
                        break;
                    }
                };
                if !status.success() {
                    break;
                }
            }
        }
        if let Some(prefix) = &passlog {
            cleanup_passlogs(prefix);
        }

        if is_cancelled(&state, idx) {
            let _ = std::fs::remove_file(&out_path);
            emit_p!(-2.0, "Cancelled");
            emit_l!("[INFO] Cancelled");
            results.push(FileResult {
                file_index: idx,
                file_name: stem,
                output_path: String::new(),
                output_size: 0,
                success: false,
                cancelled: true,
                error: None,
            });
        } else if let Some(msg) = retry_error {
            let _ = std::fs::remove_file(&out_path);
            emit_p!(-1.0, format!("Error: {msg}"));
            emit_l!(format!("[ERROR] Size-correction encode failed: {msg}"));
            results.push(FileResult {
                file_index: idx,
                file_name: stem,
                output_path: String::new(),
                output_size: 0,
                success: false,
                cancelled: false,
                error: Some(msg),
            });
        } else if status.success() {
            let sz = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            if options.algorithm == "fixed" && sz > hard_limit_bytes {
                let msg = format!(
                    "Encoder could not meet the {:.2} MB hard limit after size correction (produced {:.2} MB)",
                    options.target_size,
                    sz as f64 / 1024.0 / 1024.0
                );
                let _ = std::fs::remove_file(&out_path);
                emit_p!(-1.0, format!("Error: {msg}"));
                emit_l!(format!("[ERROR] {msg}"));
                results.push(FileResult {
                    file_index: idx,
                    file_name: stem,
                    output_path: String::new(),
                    output_size: 0,
                    success: false,
                    cancelled: false,
                    error: Some(msg),
                });
                continue;
            }
            let mb = sz as f64 / 1024.0 / 1024.0;
            emit_p!(100.0, "Done!");
            emit_l!(format!("[DONE] {} ({mb:.2} MB)", out_name));
            results.push(FileResult {
                file_index: idx,
                file_name: stem,
                output_path: out_path,
                output_size: sz,
                success: true,
                cancelled: false,
                error: None,
            });
        } else {
            let msg = format!("FFmpeg exited with code {}", status.code().unwrap_or(-1));
            emit_p!(-1.0, format!("Error: {msg}"));
            emit_l!(format!("[ERROR] {msg}"));
            results.push(FileResult {
                file_index: idx,
                file_name: stem,
                output_path: String::new(),
                output_size: 0,
                success: false,
                cancelled: false,
                error: Some(msg),
            });
        }
    }
    state.active_pids.lock().unwrap().clear();
    state.running.store(false, Ordering::SeqCst);
    Ok(results)
}

/// Minimal base64 encoder — avoids adding a crate dependency.
fn base64_encode(data: &[u8]) -> String {
    const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.len();
        let b0 = chunk[0] as usize;
        let b1 = if n > 1 { chunk[1] as usize } else { 0 };
        let b2 = if n > 2 { chunk[2] as usize } else { 0 };
        out.push(C[b0 >> 2] as char);
        out.push(C[((b0 & 3) << 4) | (b1 >> 4)] as char);
        out.push(if n > 1 {
            C[((b1 & 15) << 2) | (b2 >> 6)] as char
        } else {
            '='
        });
        out.push(if n > 2 { C[b2 & 63] as char } else { '=' });
    }
    out
}

#[tauri::command]
async fn get_thumbnail(
    state: tauri::State<'_, AppState>,
    file_path: String,
) -> Result<String, String> {
    let ffmpeg = state
        .ffmpeg
        .lock()
        .unwrap()
        .clone()
        .ok_or("FFmpeg not available")?;
    // Seek 0.5 s in to skip black openers; scale to a small thumbnail width
    let mut command = hidden_cmd(&ffmpeg);
    command
        .args([
            "-ss",
            "0.5",
            "-i",
            &file_path,
            "-vframes",
            "1",
            "-vf",
            "scale=128:-1",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-an",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    state.process_job.assign(&child);
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if out.stdout.is_empty() {
        return Err("no output".into());
    }
    Ok(format!(
        "data:image/png;base64,{}",
        base64_encode(&out.stdout)
    ))
}

#[tauri::command]
async fn open_folder(path: String) {
    let p = std::path::Path::new(&path);
    let folder = if p.is_dir() {
        path.clone()
    } else {
        p.parent()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone())
    };
    #[cfg(windows)]
    let _ = hidden_std_cmd("explorer").arg(&folder).spawn();
}

#[tauri::command]
fn set_minimize_to_tray(state: tauri::State<'_, AppState>, enabled: bool) {
    state.minimize_to_tray.store(enabled, Ordering::SeqCst);
}

#[tauri::command]
fn request_titlebar_close(state: tauri::State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    if state.running.load(Ordering::SeqCst) {
        app.emit("close-requested-during-compression", ())
            .map_err(|error| error.to_string())?;
    } else if state.minimize_to_tray.load(Ordering::SeqCst) {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "Main window is unavailable".to_string())?;
        window.hide().map_err(|error| error.to_string())?;
    } else {
        state.allow_close.store(true, Ordering::SeqCst);
        app.exit(0);
    }
    Ok(())
}

#[tauri::command]
async fn play_sound(state: tauri::State<'_, AppState>) -> Result<(), String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let wav = dir.join("finish.wav");
            if wav.exists() {
                let mut command = hidden_cmd("powershell");
                command.args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "(New-Object System.Media.SoundPlayer '{}').PlaySync()",
                        wav.to_string_lossy()
                    ),
                ]);
                if let Ok(child) = command.spawn() {
                    #[cfg(windows)]
                    state.process_job.assign(&child);
                }
            }
        }
    }
    Ok(())
}

fn settings_file_path() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let folder = exe
        .parent()
        .ok_or_else(|| "Unable to locate the application folder".to_string())?;
    Ok(folder.join("effigy-video-compressor.cfg"))
}

fn ensure_settings_file() -> Result<(), String> {
    let path = settings_file_path()?;
    if path.exists() {
        return Ok(());
    }
    let defaults = serde_json::json!({
        "version": 6,
        "codec": "libx264", "codec_name": "x264", "crf": 24,
        "container": "mp4", "algorithm": "dynamic",
        "res_mode": "auto", "res_w": null, "res_h": null,
        "resolution_name": "auto", "fps": 0, "fps_name": "auto",
        "target_size": 8, "size_name": "8mb", "allocation_mode": "lightweight", "preset": "slower",
        "svt_preset": 8,
        "uiTheme": "studio", "accent": "rose", "customAccent": "#22D3EE",
        "mode": "dark", "openFolder": false, "rememberSettings": true,
        "startOnAdd": false, "minimizeToTray": false
    });
    let contents = serde_json::to_string_pretty(&defaults).map_err(|error| error.to_string())?;
    std::fs::write(path, contents).map_err(|error| error.to_string())
}

#[tauri::command]
fn load_settings() -> Result<Option<serde_json::Value>, String> {
    let path = settings_file_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|error| format!("Invalid {}: {error}", path.display()))
}

#[tauri::command]
fn save_settings(settings: serde_json::Value) -> Result<(), String> {
    let path = settings_file_path()?;
    let contents = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    std::fs::write(path, contents).map_err(|error| error.to_string())
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn restore_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // This must be the first plugin. A second EXE launch is redirected here
        // and the already-running window is restored, including from the tray.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            restore_main_window(app);
        }))
        .manage(AppState {
            ffmpeg: Mutex::new(None),
            ffprobe: Mutex::new(None),
            active_pids: Mutex::new(HashMap::new()),
            cancelled_files: Mutex::new(HashSet::new()),
            cancel_all: AtomicBool::new(false),
            running: AtomicBool::new(false),
            allow_close: AtomicBool::new(false),
            minimize_to_tray: AtomicBool::new(false),
            #[cfg(windows)]
            process_job: ProcessJob::new(),
        })
        .setup(|app| {
            if let Err(error) = ensure_settings_file() {
                eprintln!("Unable to create effigy-video-compressor.cfg: {error}");
            }

            let show = MenuItem::with_id(app, "show", "Show Effigy", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Effigy Video Compressor")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id() == "show" {
                        restore_main_window(app);
                    } else if event.id() == "quit" {
                        let state = app.state::<AppState>();
                        state.allow_close.store(true, Ordering::SeqCst);
                        state.cancel_all.store(true, Ordering::SeqCst);
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        restore_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            check_ffmpeg,
            detect_hardware_encoders,
            select_files,
            select_output_folder,
            compress_files,
            cancel_file,
            cancel_all_compressions,
            cancel_all_and_close,
            install_ffmpeg,
            get_thumbnail,
            open_folder,
            set_minimize_to_tray,
            request_titlebar_close,
            play_sound,
            load_settings,
            save_settings,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.running.load(Ordering::SeqCst) && !state.allow_close.load(Ordering::SeqCst)
                {
                    api.prevent_close();
                    let _ = window.emit("close-requested-during-compression", ());
                } else if state.minimize_to_tray.load(Ordering::SeqCst)
                    && !state.allow_close.load(Ordering::SeqCst)
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

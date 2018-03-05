#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate env_logger;

// use std::env;
use std::fs::{self};
use std::mem::swap;
use std::process::{self, Command};
use std::sync::RwLock;
use std::thread;
use streams::*;

mod h264;
mod http;
mod streams;

const STREAM_TMP_DIR: &'static str = "/tmp/raspivid-stream";
const FRAMERATE: usize = 20;

struct Singleton<T>(T);

lazy_static! {
	static ref STREAM_FILE_COUNTER: RwLock<Singleton<usize>> = RwLock::new(Singleton(0));
}

fn main() {
	env_logger::init();
	clean_tmp_dir();

	http::init_iron();

	let mut ffmpeg = FFMpeg::spawn();
	loop {
		let mut child = if let Ok(child) = Command::new("raspivid")
			.args(vec!["-o", "-"]) // Output to STDOUT
			.args(vec!["-t", "7200000"]) // Stay on for a 2 hours instead of quickly exiting
			.args(vec!["-rot", "90"]) // Rotate 90 degrees as the device is sitting sideways.
			.args(vec!["-w", "1280"]) // Width
			.args(vec!["-h", "720"]) // Height
			.args(vec!["-fps", &format!("{}", FRAMERATE)]) // Framerate
//			.args(vec!["-a", "4"]) // Output time
//			.args(vec!["-a", &format!("Device: {} | %F %X %Z", env::var("HOSTNAME").unwrap_or("unknown".to_string()))]) // Supplementary argument hmm rn it requires an additional `export` command
			.stdin(process::Stdio::null())
			.stdout(process::Stdio::piped())
			.spawn() { child } else { panic!("Failed to spawn raspivid process."); };
		info!("Loaded raspivid instance.");

		let mut child_stdout = child.stdout.take().unwrap_or_else(|| {
			let _ = child.kill();
			panic!("Failed to attach to raspivid's STDOUT")
		});

		let mut pic_param = vec![];
		let mut seq_param = vec![];

		while let Ok(None) = child.try_wait() {
			h264::split_stream(&mut child_stdout, &mut ffmpeg, &mut pic_param, &mut seq_param, |mut frame: Vec<u8>, ffmpeg: &mut FFMpeg, mut pic_param: &mut Vec<u8>, mut seq_param: &mut Vec<u8>| {
				let unit_type = h264::get_unit_type(&frame);
				loop {
					match unit_type {
						5 => {
							// Minimum 4 seconds buffer
							if ffmpeg.is_saturated() {
								let mut handle = FFMpeg::spawn();

								handle.write(&mut pic_param);
								handle.write(&mut seq_param);

								swap(ffmpeg, &mut handle);
								let _ = thread::Builder::new().name("ffmpeg handle".to_string()).spawn(move || {
									let counter = {
										let mut ptr = STREAM_FILE_COUNTER.write().unwrap();
										ptr.0 += 1;
										ptr.0
									};
									handle.process();

									let path = format!("{}/{}", STREAM_TMP_DIR, counter);
									let _ = fs::rename(&format!("{}/stream_replace.mp4", STREAM_TMP_DIR), &path);
									info!("Outputted new mp4 file at {}", path);

									if counter >= 4 {
										let _ = fs::remove_file(&format!("{}/{}", STREAM_TMP_DIR, counter - 4)); // Delete old
									}
								});
							}
						}
						7 => pic_param.extend(&frame[..]),
						8 => seq_param.extend(&frame[..]),
						_ => {}
					}
					break;
				}
				ffmpeg.write(&mut frame);
			});
		}
	}
}

fn clean_tmp_dir() {
	let _ = fs::remove_dir_all(STREAM_TMP_DIR);
	let _ = fs::create_dir(STREAM_TMP_DIR);
}

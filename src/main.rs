#![feature(drain_filter)]

use serde::Deserialize;
use structopt::StructOpt;
use structopt::clap::AppSettings;
use std::io::BufRead;
use std::os::unix::process::ExitStatusExt;

#[derive(StructOpt)]
#[structopt(setting = AppSettings::ColoredHelp)]
#[structopt(setting = AppSettings::UnifiedHelpMessage)]
#[structopt(setting = AppSettings::DeriveDisplayOrder)]
struct Options {
	/// The command to run when a message is received.
	#[structopt(long)]
	#[structopt(value_name = "COMMAND")]
	action: String,

	/// Clear the environment of the action child process.
	#[structopt(long)]
	clear_env: bool,

	/// The command to run the rtl_433 tool.
	#[structopt(long)]
	#[structopt(default_value = "rtl_433")]
	#[structopt(value_name = "COMMAND")]
	rtl433_bin: String,

	/// The device for rtl_433 to connect to.
	device: Option<String>,

	/// Filter on group.
	#[structopt(long, short)]
	#[structopt(value_name = "GROUP")]
	group: Option<u32>,

	/// Filter on unit.
	#[structopt(long, short)]
	#[structopt(value_name = "UNIT")]
	unit: Option<u32>,

	/// Filter on ID.
	#[structopt(long, short)]
	#[structopt(value_name = "ID")]
	id: Option<u32>,

	/// Filter on channel.
	#[structopt(long, short)]
	#[structopt(value_name = "CHANNEL")]
	channel: Option<u32>,
}

struct Application {
	child: Option<std::process::Child>,
	actions: Vec<std::process::Child>,
}

impl Application {
	fn new() -> Self {
		Self {
			child: None,
			actions: Vec::new(),
		}
	}

	fn run(&mut self, options: &Options) -> Result<(), String> {
		let mut command = std::process::Command::new(&options.rtl433_bin);
		command.stdin(std::process::Stdio::null());
		command.stdout(std::process::Stdio::piped());
		command.stderr(std::process::Stdio::inherit());
		command.args(&[
			"-F", "json",
			"-M", "newmodel",
			"-R", "51",
		]);

		if let Some(device) = &options.device {
			command.args(&["-d", device]);
		}

		self.child = Some(command.spawn()
			.map_err(|e| format!("Failed to run {:?}: {}", options.rtl433_bin, e))?);

		let stream = self.child.as_mut().unwrap().stdout.as_mut().ok_or("No stdout available from child process.")?;

		let stream = std::io::BufReader::new(stream);
		let mut action : Option<std::process::Child> = None;

		for message in stream.lines() {
			let message = message
				.map_err(|e| format!("Failed to read message from child: {}", e))?;

			let event = serde_json::from_str::<Event>(&message)
				.map_err(|e| format!("Failed to parse message from child: {}", e))?;

			if options.group.as_ref().map(|x| *x == event.group) == Some(false) {
				continue;
			}

			if options.unit.as_ref().map(|x| *x == event.unit) == Some(false) {
				continue;
			}

			if options.id.as_ref().map(|x| *x == event.id) == Some(false) {
				continue;
			}

			if options.channel.as_ref().map(|x| *x == event.channel) == Some(false) {
				continue;
			}

			if let Some(action) = &mut action {
				let _ = action.kill();
			}

			let mut new_action = std::process::Command::new(&options.action);
			if options.clear_env {
				new_action.env_clear();
			}

			new_action.env("TIME",    &event.time);
			new_action.env("MODEL",   &event.model);
			new_action.env("GROUP",   format!("{}", event.group));
			new_action.env("UNIT",    format!("{}", event.unit));
			new_action.env("ID",      format!("{}", event.id));
			new_action.env("CHANNEL", format!("{}", event.channel));
			new_action.env("STATE",   if event.state { "1" } else { "0" });
			let new_action = new_action.spawn().map_err(|e| format!("Failed to run action: {}", e))?;
			self.actions.push(new_action);
			clear_actions(&mut self.actions);
		}

		Ok(())
	}
}

fn clear_actions(actions: &mut Vec<std::process::Child>) {
	actions.drain_filter(|action| match action.try_wait() {
		Ok(None) => true,
		Ok(Some(x)) => {
			log_status_code("Action", Ok(x));
			false
		},
		Err(e) => {
			log_status_code("Action", Err(e));
			false
		},
	}).count();
}

fn log_status_code(name: &str, status: Result<std::process::ExitStatus, std::io::Error>) -> bool {
	let status = match status {
		Ok(x) => x,
		Err(e) => {
			eprintln!("Failed to determine exit status of {}: {}", name, e);
			return true;
		}
	};

	match (status.code(), status.signal()) {
		(Some(0), None) => false,
		(Some(code), None) => {
			eprintln!("{} exitted with status {}", name, code);
			true
		},
		(None, Some(signal)) => {
			eprintln!("{} killed by signal {}", name, signal);
			true
		},
		_ => {
			eprintln!("{} exitted with unknown error condition", name);
			true
		}
	}
}

fn main() {
	let options = Options::from_args();
	let mut app = Application::new();
	let mut error = false;
	let result = app.run(&options);

	if let Err(e) = result {
		eprintln!("{}", e);
		error |= true;
	}

	if let Some(child) = &mut app.child {
		let _ = child.kill();
		error |= log_status_code(&options.rtl433_bin, child.wait());
	}

	if error {
		std::process::exit(1);
	}
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct Event {
	pub time: String,
	pub model: String,
	pub group: u32,
	pub unit: u32,
	pub id: u32,
	pub channel: u32,
	#[serde(deserialize_with = "deserialize_state")]
	pub state: bool,
}

fn deserialize_state<'de, D>(de: D) -> Result<bool, D::Error>
where
	D: serde::Deserializer<'de>
{
	let state : &str = Deserialize::deserialize(de)?;
	match state {
		"ON"  => Ok(true),
		"OFF" => Ok(false),
		x => Err(serde::de::Error::invalid_value(serde::de::Unexpected::Str(x), &"ON or OFF")),
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn test_decode() {
		const SAMPLE : &str = "{\"time\" : \"2019-12-11 15:21:51\", \"model\" : \"Proove-Security\", \"id\" : 3, \"channel\" : 4, \"state\" : \"ON\", \"unit\" : 2, \"group\" : 1}";
		let event = serde_json::from_str::<Event>(SAMPLE).expect("failed to parse JSON");
		assert_eq!(event.time, "2019-12-11 15:21:51");
		assert_eq!(event.model, "Proove-Security");
		assert_eq!(event.group, 1);
		assert_eq!(event.unit, 2);
		assert_eq!(event.id, 3);
		assert_eq!(event.channel, 4);
		assert_eq!(event.state, true);
	}
}

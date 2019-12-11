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

fn main() {
	let options = Options::from_args();

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

	let mut child = match command.spawn() {
		Ok(x) => x,
		Err(error) => {
			eprintln!("Failed to run {:?}: {}", options.rtl433_bin, error);
			std::process::exit(1);
		}
	};

	let stream = match &mut child.stdout {
		Some(x) => x,
		None => {
			eprintln!("No stdout available from child process.");
			std::process::exit(1);
		}
	};

	let stream = std::io::BufReader::new(stream);
	let mut action : Option<std::process::Child> = None;

	for message in stream.lines() {
		let message = match message {
			Ok(x) => x,
			Err(e) => {
				eprintln!("Failed to read message from child: {}", e);
				std::process::exit(1);
			}
		};

		let event = match serde_json::from_str::<Event>(&message) {
			Ok(x) => x,
			Err(error) => {
				eprintln!("Failed to parse message from child: {}", error);
				std::process::exit(1);
			}
		};

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
		let new_action = new_action.spawn();

		match new_action {
			Ok(x) => action = Some(x),
			Err(error) => {
				println!("Failed to run action: {}", error);
				std::process::exit(1);
			}
		}
	}

	let _ = child.kill();
	let status = child.wait().unwrap();
	if let Some(code) = status.code() {
		eprintln!("rtl_433 exitted with status {}", code);
		std::process::exit(1);
	} else if let Some(signal) = status.signal() {
		eprintln!("rtl_433 killed by signal {}", signal);
		std::process::exit(1);
	} else if !status.success() {
		eprintln!("rtl_433 exitted with unknown error condition");
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

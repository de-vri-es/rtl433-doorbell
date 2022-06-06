use std::collections::BTreeMap;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::rc::Rc;
use structopt::StructOpt;
use structopt::clap::AppSettings;
use tokio::io::AsyncBufReadExt;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

mod event;
use event::Event;

pub mod server;

#[derive(StructOpt)]
#[structopt(setting = AppSettings::ColoredHelp)]
#[structopt(setting = AppSettings::UnifiedHelpMessage)]
#[structopt(setting = AppSettings::DeriveDisplayOrder)]
#[structopt(setting = AppSettings::TrailingVarArg)]
struct Options {
	/// The command to run when a message is received.
	#[structopt(value_name = "COMMAND")]
	#[structopt(required = true)]
	action: String,

	/// Arguments to the command.
	#[structopt(value_name = "ARGS")]
	args: Vec<String>,

	/// Kill a running action if a new one is triggered.
	#[structopt(long)]
	#[structopt(conflicts_with = "skip-busy")]
	kill_busy: bool,

	/// Ignore new actions if an old one is still running.
	#[structopt(long)]
	skip_busy: bool,

	/// Clear the environment of the action child process.
	#[structopt(long)]
	clear_env: bool,

	/// The command to run the rtl_433 tool.
	#[structopt(long)]
	#[structopt(default_value = "rtl_433")]
	#[structopt(value_name = "COMMAND")]
	rtl433_bin: String,

	/// The device for rtl_433 to connect to.
	#[structopt(long)]
	#[structopt(value_name = "DEVICE")]
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
	let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
	let local = tokio::task::LocalSet::new();

	let options = Options::from_args();

	let mut error = false;
	let result = local.block_on(&rt, async {
		let app = match Application::new(options) {
			Ok(x) => x,
			Err(e) => {
				eprintln!("{}", e);
				std::process::exit(1);
			},
		};
		app.run().await
	});

	if let Err(e) = result {
		eprintln!("{}", e);
		error |= true;
	}

	// for action in &mut app.actions {
	// 	let _ = action.kill();
	// 	log_status_code("Action", action.await);
	// }

	if error {
		std::process::exit(1);
	}
}

struct Application {
	options: Options,
	child: Mutex<Child>,
	actions: Mutex<BTreeMap<u32, JoinHandle<()>>>,
}

impl Application {
	fn new(options: Options) -> Result<Rc<Self>, String> {
		let mut command = Command::new(&options.rtl433_bin);
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

		let child = command.spawn().map_err(|e| format!("Failed to run {:?}: {}", options.rtl433_bin, e))?;

		Ok(Rc::new(Self {
			options,
			child: Mutex::new(child),
			actions: Mutex::new(BTreeMap::new()),
		}))
	}

	async fn run(self: Rc<Self>) -> Result<(), String> {
		let mut child = self.child.lock().await;

		let stream = child.stdout.as_mut().ok_or("No stdout available from child process.")?;
		let stream = tokio::io::BufReader::new(stream);
		let mut lines = stream.lines();

		while let Some(message) = lines.next_line().await.map_err(|e| format!("Failed to read message from child: {}", e))? {
			let event = serde_json::from_str::<Event>(&message)
				.map_err(|e| format!("Failed to parse message from child: {}", e))?;

			if self.options.group.as_ref().map(|x| *x == event.group) == Some(false) {
				continue;
			}

			if self.options.unit.as_ref().map(|x| *x == event.unit) == Some(false) {
				continue;
			}

			if self.options.id.as_ref().map(|x| *x == event.id) == Some(false) {
				continue;
			}

			if self.options.channel.as_ref().map(|x| *x == event.channel) == Some(false) {
				continue;
			}

			if let Err(e) = self.clone().run_action(&event).await {
				eprintln!("{}", e);
			}
		}

		Ok(())
	}

	async fn run_action(self: Rc<Self>, event: &Event) -> Result<(), String> {
		if self.options.skip_busy {
			let actions = self.actions.lock().await;
			if !actions.is_empty() {
				eprintln!("Previous action is still running, ignoring event.");
				return Ok(())
			}
		}

		if self.options.kill_busy {
			loop {
				let (pid, join) = {
					let mut actions = self.actions.lock().await;
					let pid = match actions.iter().next() {
						None => break,
						Some((pid, _)) => *pid,
					};
					(pid, actions.remove(&pid).unwrap())
				};
				eprintln!("Previous action is still running, killing process {}.", pid);
				kill(pid, libc::SIGTERM);
				join.await.unwrap();
			}
		}

		let mut action = Command::new(&self.options.action);
		if self.options.clear_env {
			action.env_clear();
		}

		action.args(&self.options.args);
		action.env("TIME",    &event.time);
		action.env("MODEL",   &event.model);
		action.env("GROUP",   format!("{}", event.group));
		action.env("UNIT",    format!("{}", event.unit));
		action.env("ID",      format!("{}", event.id));
		action.env("CHANNEL", format!("{}", event.channel));
		action.env("STATE",   if event.state { "1" } else { "0" });

		let mut child = action
			.spawn()
			.map_err(|e| format!("Failed to run action: {}", e))?;
		let pid = child.id()
			.ok_or("Failed to get PID of child")?;

		let this = self.clone();
		let join = tokio::task::spawn_local(async move {
			let status = child.wait().await;
			log_status_code("action", status);
			let mut actions = this.actions.lock().await;
			actions.remove(&pid);
		});

		let mut actions = self.actions.lock().await;
		actions.entry(pid)
			.and_modify(|_| panic!("PID already in map: {}", pid))
			.or_insert(join);

		Ok(())
	}
}

fn log_status_code(name: &str, status: Result<ExitStatus, std::io::Error>) {
	let status = match status {
		Ok(x) => x,
		Err(e) => {
			eprintln!("Failed to determine exit status of {}: {}", name, e);
			return;
		},
	};

	match (status.code(), status.signal()) {
		(Some(0), None) => (),
		(Some(code), None) => eprintln!("{} exitted with status {}", name, code),
		(None, Some(signal)) => eprintln!("{} killed by signal {}", name, signal),
		_ => eprintln!("{} exitted with unknown error condition", name),
	}
}

fn kill(pid: u32, signal: i32) {
	unsafe { libc::kill(pid as libc::pid_t, signal as libc::c_int) };
}

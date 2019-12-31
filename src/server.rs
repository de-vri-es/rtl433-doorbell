use std::future::Future;
use std::net::SocketAddr;
use tokio::io::AsyncBufRead;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::BufWriter;
use tokio::net::TcpListener;
use tokio::net::ToSocketAddrs;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

pub struct Server {
	listener: TcpListener,
	events: broadcast::Sender<Message>,
	tasks: Vec<JoinHandle<std::io::Result<()>>>,
}

#[derive(Clone, Debug)]
pub struct Broadcaster {
	sender: broadcast::Sender<Message>,
}

#[derive(Clone, Debug)]
enum Source {
	Internal,
	Socket(SocketAddr),
}

#[derive(Clone, Debug)]
enum Message {
	DingDong(Source),
}


impl Server {
	pub fn new(listener: TcpListener) -> Self {
		let (events, _) = broadcast::channel(10);
		Self {
			listener,
			events,
			tasks: Vec::new(),
		}
	}

	pub async fn bind(address: impl ToSocketAddrs) -> std::io::Result<Self> {
		let listener = TcpListener::bind(address).await?;
		Ok(Self::new(listener))
	}

	pub async fn run(&mut self) -> std::io::Result<()> {
		loop {
			self.accept_one().await?;
		}
	}

	pub fn broadcaster(&self) -> Broadcaster {
		Broadcaster {
			sender: self.events.clone(),
		}
	}

	fn spawn<Fut>(&mut self, future: Fut)
	where
		Fut: 'static + Future<Output = std::io::Result<()>>,
	{
		let handle = tokio::task::spawn_local(future);
		self.tasks.push(handle);
	}

	async fn accept_one(&mut self) -> std::io::Result<()> {
		let (stream, address) = self.listener.accept().await?;
		let (read, write) = tokio::io::split(stream);

		self.spawn(Self::run_read_loop(address, BufReader::new(read), self.events.clone()));
		self.spawn(Self::run_write_loop(address, BufWriter::new(write), self.events.subscribe()));

		Ok(())
	}

	async fn run_read_loop(
		address: SocketAddr,
		read: impl AsyncBufRead + Unpin,
		sender: broadcast::Sender<Message>
	) -> std::io::Result<()> {
		let mut lines = read.lines();
		while let Some(line) = lines.next_line().await? {
			match line.as_str() {
				"dingdong" => {
					let _ = sender.send(Message::DingDong(Source::Socket(address)));
				},
				_ => (),
			}
		}

		Ok(())
	}

	async fn run_write_loop(
		_address: SocketAddr,
		mut write: impl AsyncWrite + Unpin,
		mut receiver: broadcast::Receiver<Message>
	) -> std::io::Result<()> {
		loop {
			let message = match receiver.recv().await {
				Err(broadcast::RecvError::Closed) => break,
				Err(broadcast::RecvError::Lagged{..}) => continue,
				Ok(address) => address,
			};

			match message {
				Message::DingDong(_) => {
					write.write_all(b"dingdong\n").await?;
					write.flush().await?;
				},
			}
		}
		Ok(())
	}
}

impl Broadcaster {
	pub fn send_ding_dong(&self) {
		let _ = self.sender.send(Message::DingDong(Source::Internal));
	}
}

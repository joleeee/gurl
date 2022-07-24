use mio::net::TcpStream;
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};
use url::{Host, Url};

mod cert;
use cert::NoVerify;
mod response;
use response::Response;

#[derive(Debug)]
pub enum AgentError {
    UrlError,
}

pub struct Agent;

impl Agent {
    pub fn get(url: Url) -> Result<Request, AgentError> {
        let host = url.host().ok_or(AgentError::UrlError)?;

        Ok(Request {
            host: host.to_owned(),
            url,
            port: 1965,
        })
    }
}

#[derive(Debug)]
pub struct Request {
    host: Host,
    url: Url,
    port: u16,
}

const CLIENT: mio::Token = mio::Token(0);

struct GeminiClient {
    socket: TcpStream,
    tls_conn: rustls::ClientConnection,
    closing: bool,
    clean_closure: bool,
    output: Vec<u8>,
}

impl GeminiClient {
    fn new(
        socket: TcpStream,
        server_name: rustls::ServerName,
        cfg: Arc<rustls::ClientConfig>,
    ) -> Self {
        Self {
            socket,
            tls_conn: rustls::ClientConnection::new(cfg, server_name).unwrap(),
            closing: false,
            clean_closure: false,
            output: Vec::new(),
        }
    }

    /// Consumes self
    fn request(mut self, url: &Url) -> Response {
        let request = url.to_string() + "\r\n";
        self.tls_conn
            .writer()
            .write_all(request.as_bytes())
            .unwrap();

        let mut poll = mio::Poll::new().unwrap();
        let mut events = mio::Events::with_capacity(8);
        self.register(poll.registry());

        loop {
            poll.poll(&mut events, None).unwrap();

            for ev in events.iter() {
                self.ready(ev);
                self.reregister(poll.registry());
            }

            if self.is_closed() {
                break;
            }
        }

        Response::from_raw(&self.output).unwrap()
    }

    fn ready(&mut self, ev: &mio::event::Event) {
        assert_eq!(ev.token(), CLIENT);

        if ev.is_readable() {
            self.do_read();
        }

        if ev.is_writable() {
            self.do_write();
        }
    }

    fn do_read(&mut self) {
        match self.tls_conn.read_tls(&mut self.socket) {
            Err(err) => {
                if err.kind() == io::ErrorKind::WouldBlock {
                    return;
                }
                self.closing = true;
                return;
            }

            Ok(0) => {
                self.closing = true;
                self.clean_closure = true;
                return;
            }

            Ok(_) => {}
        }

        let io_state = match self.tls_conn.process_new_packets() {
            Ok(s) => s,
            Err(err) => {
                eprintln!("err {}", err);
                self.closing = true;
                return;
            }
        };

        if io_state.plaintext_bytes_to_read() > 0 {
            let mut data = vec![0; io_state.plaintext_bytes_to_read()];
            self.tls_conn.reader().read_exact(&mut data).unwrap();
            self.output.extend_from_slice(&data);
        }

        if io_state.peer_has_closed() {
            self.clean_closure = true;
            self.closing = true;
        }
    }

    fn do_write(&mut self) {
        self.tls_conn.write_tls(&mut self.socket).unwrap();
    }

    fn is_closed(&self) -> bool {
        self.closing
    }

    fn register(&mut self, registry: &mio::Registry) {
        let interest = self.event_set();
        registry
            .register(&mut self.socket, CLIENT, interest)
            .unwrap();
    }

    fn reregister(&mut self, registry: &mio::Registry) {
        let interest = self.event_set();
        registry
            .reregister(&mut self.socket, CLIENT, interest)
            .unwrap();
    }

    fn event_set(&self) -> mio::Interest {
        let rd = self.tls_conn.wants_read();
        let wr = self.tls_conn.wants_write();

        if rd && wr {
            mio::Interest::READABLE | mio::Interest::WRITABLE
        } else if wr {
            mio::Interest::WRITABLE
        } else {
            mio::Interest::READABLE
        }
    }
}

impl Request {
    pub fn run(&self) -> Response {
        let config = {
            let verifier = NoVerify;

            let config = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();

            Arc::new(config)
        };

        let name = self.host.to_string().as_str().try_into().unwrap();
        let address = {
            let mut adrs = format!("{}:{}", self.host, self.port)
                .to_socket_addrs()
                .unwrap();
            SocketAddr::from(adrs.next().unwrap())
        };
        let sock = TcpStream::connect(address).unwrap();

        let gem = GeminiClient::new(sock, name, config);

        gem.request(&self.url)
    }
}

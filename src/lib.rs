use mio::net::TcpStream;
use std::{
    io::{self, Read, Write},
    net::ToSocketAddrs,
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

#[derive(Debug)]
pub enum RequestError {
    DecodeError(response::ResponseError),
    IoError(io::Error),
    TlsError(rustls::Error),
}

impl From<response::ResponseError> for RequestError {
    fn from(e: response::ResponseError) -> Self {
        Self::DecodeError(e)
    }
}

impl From<io::Error> for RequestError {
    fn from(e: io::Error) -> Self {
        Self::IoError(e)
    }
}

impl From<rustls::Error> for RequestError {
    fn from(e: rustls::Error) -> Self {
        Self::TlsError(e)
    }
}

impl GeminiClient {
    fn new(
        socket: TcpStream,
        server_name: rustls::ServerName,
        cfg: Arc<rustls::ClientConfig>,
    ) -> Result<Self, RequestError> {
        Ok(Self {
            socket,
            tls_conn: rustls::ClientConnection::new(cfg, server_name)?,
            closing: false,
            clean_closure: false,
            output: Vec::new(),
        })
    }

    /// Consumes self
    fn request(mut self, url: &Url) -> Result<Response, RequestError> {
        let request = url.to_string() + "\r\n";
        self.tls_conn.writer().write_all(request.as_bytes())?;

        let mut poll = mio::Poll::new()?;
        let mut events = mio::Events::with_capacity(8);
        self.register(poll.registry())?;

        loop {
            poll.poll(&mut events, None)?;

            for ev in events.iter() {
                self.ready(ev)?;
                self.reregister(poll.registry())?;
            }

            if self.is_closed() {
                break;
            }
        }

        Ok(Response::from_raw(&self.output)?)
    }

    fn ready(&mut self, ev: &mio::event::Event) -> Result<(), RequestError> {
        assert_eq!(ev.token(), CLIENT);

        if ev.is_readable() {
            self.do_read()?;
        }

        if ev.is_writable() {
            self.do_write()?;
        }

        Ok(())
    }

    fn do_read(&mut self) -> Result<(), RequestError> {
        if self.tls_conn.read_tls(&mut self.socket)? == 0 {
            // EOF
            self.closing = true;
            self.clean_closure = true;
            return Ok(());
        }

        let io_state = self.tls_conn.process_new_packets()?;

        if io_state.plaintext_bytes_to_read() > 0 {
            let mut data = vec![0; io_state.plaintext_bytes_to_read()];
            self.tls_conn
                .reader()
                .read_exact(&mut data)
                .expect("read exact should never fail");
            self.output.extend_from_slice(&data);
        }

        if io_state.peer_has_closed() {
            self.clean_closure = true;
            self.closing = true;
        }

        Ok(())
    }

    fn do_write(&mut self) -> Result<usize, RequestError> {
        Ok(self.tls_conn.write_tls(&mut self.socket)?)
    }

    fn is_closed(&self) -> bool {
        self.closing
    }

    fn register(&mut self, registry: &mio::Registry) -> Result<(), RequestError> {
        let interest = self.event_set();
        registry.register(&mut self.socket, CLIENT, interest)?;
        Ok(())
    }

    fn reregister(&mut self, registry: &mio::Registry) -> Result<(), RequestError> {
        let interest = self.event_set();
        registry.reregister(&mut self.socket, CLIENT, interest)?;
        Ok(())
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
            adrs.next().unwrap()
        };
        let sock = TcpStream::connect(address).unwrap();

        let gem = GeminiClient::new(sock, name, config).unwrap();

        gem.request(&self.url).unwrap()
    }
}

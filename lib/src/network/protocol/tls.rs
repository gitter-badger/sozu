use mio::*;
use mio::tcp::*;
use pool::Checkout;
use network::buffer_queue::BufferQueue;
use openssl::ssl::{self,HandshakeError,MidHandshakeSslStream,Ssl,SslStream};
use network::{ClientResult,Protocol};
use network::proxy::Readiness;
use network::protocol::ProtocolResult;

pub enum TlsState {
  Initial,
  Handshake,
  Established,
  Error,
}

pub struct TlsHandshake {
  pub readiness:       Readiness,
  pub front:           Option<TcpStream>,
  pub ssl:             Option<Ssl>,
  pub stream:          Option<SslStream<TcpStream>>,
  pub server_context:  String,
  mid:                 Option<MidHandshakeSslStream<TcpStream>>,
  state:               TlsState,
}

impl TlsHandshake {
  pub fn new(server_context: &str, ssl:Ssl, sock: TcpStream) -> TlsHandshake {
    TlsHandshake {
      front:          Some(sock),
      server_context: String::from(server_context),
      ssl:            Some(ssl),
      mid:            None,
      stream:         None,
      state:          TlsState::Initial,
      readiness:      Readiness {
                        front_interest:  Ready::readable() | Ready::hup() | Ready::error(),
                        back_interest:   Ready::none(),
                        front_readiness: Ready::none(),
                        back_readiness:  Ready::none(),
      },
    }
  }

  pub fn readable(&mut self) -> (ProtocolResult,ClientResult) {
    match self.state {
      TlsState::Error   => return (ProtocolResult::Continue, ClientResult::CloseClient),
      TlsState::Initial => {
        let ssl     = self.ssl.take().unwrap();
        let sock    = self.front.take().unwrap();
        let version = ssl.version();
        match ssl.accept(sock) {
          Ok(stream) => {
            self.stream = Some(stream);
            return (ProtocolResult::Upgrade, ClientResult::Continue);
          },
          Err(HandshakeError::SetupFailure(e)) => {
            println!("accept: handshake setup failed: {:?}", e);
            println!("version: {:?}", version);
            self.state = TlsState::Error;
            return (ProtocolResult::Continue, ClientResult::CloseClient);
          },
          Err(HandshakeError::Failure(e)) => {
            println!("accept: handshake failed: {:?}", e);
            println!("version: {:?}", version);
            self.state = TlsState::Error;
            return (ProtocolResult::Continue, ClientResult::CloseClient);
          },
          Err(HandshakeError::Interrupted(mid)) => {
            self.state = TlsState::Handshake;
            self.mid = Some(mid);
            self.readiness.front_readiness.remove(Ready::readable());
            return (ProtocolResult::Continue, ClientResult::Continue);
          }
        }
      },
      TlsState::Handshake => {
        let mid = self.mid.take().unwrap();
        let version = mid.ssl().version();
        match mid.handshake() {
          Ok(stream) => {
            self.stream = Some(stream);
            return (ProtocolResult::Upgrade, ClientResult::Continue);
          },
          Err(HandshakeError::SetupFailure(e)) => {
            println!("mid handshake setup failed: {:?}", e);
            println!("version: {:?}", version);
            self.state = TlsState::Error;
            return (ProtocolResult::Continue, ClientResult::CloseClient);
          },
          Err(HandshakeError::Failure(e)) => {
            println!("mid handshake failed: {:?}", e);
            println!("version: {:?}", version);
            self.state = TlsState::Error;
            return (ProtocolResult::Continue, ClientResult::CloseClient);
          },
          Err(HandshakeError::Interrupted(new_mid)) => {
            self.state = TlsState::Handshake;
            self.mid = Some(new_mid);
            self.readiness.front_readiness.remove(Ready::readable());
            return (ProtocolResult::Continue, ClientResult::Continue);
          }
        }
      },
      TlsState::Established => {
        return (ProtocolResult::Upgrade, ClientResult::Continue);
      }
    }

  }

  fn protocol(&self)           -> Protocol {
    Protocol::TLS
  }
}


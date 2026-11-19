//! TLS client with bearer-token authentication.

use tonic::codegen::InterceptedService;
use tonic::service::Interceptor;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tonic::{Request, Status};

use crate::v1::ferrent_client::FerrentClient;

/// Client type with the token interceptor installed.
pub type AuthClient = FerrentClient<InterceptedService<Channel, TokenInterceptor>>;

/// Adds `authorization: Bearer <token>` to every request.
#[derive(Clone)]
pub struct TokenInterceptor {
    token: String,
}

impl TokenInterceptor {
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

impl Interceptor for TokenInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let value = format!("Bearer {}", self.token)
            .parse()
            .map_err(|_| Status::internal("invalid bearer token"))?;
        request.metadata_mut().insert("authorization", value);
        Ok(request)
    }
}

/// Connects over TLS, trusting only `ca_pem` (the daemon's self-signed cert).
///
/// A certificate change therefore fails the handshake: this is the pinning
/// mechanism used by the native clients.
pub async fn connect(
    uri: &str,
    ca_pem: &str,
    token: &str,
) -> Result<AuthClient, Box<dyn std::error::Error + Send + Sync>> {
    let channel = Channel::from_shared(uri.to_owned())?
        .tls_config(ClientTlsConfig::new().ca_certificate(Certificate::from_pem(ca_pem)))?
        .connect()
        .await?;
    Ok(FerrentClient::with_interceptor(
        channel,
        TokenInterceptor::new(token),
    ))
}

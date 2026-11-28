//! LocalSend v2.1 wire types, pure-Rust self-signed TLS identity, minimal HTTP/1.1 framing,
//! and peer `/register` client (docs/design/rom-drop.md).

use p256::ecdsa::{DerSignature, Signature, SigningKey};
use p256::pkcs8::EncodePrivateKey;
use rand_core::{OsRng, RngCore};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, KeyPair, RemoteKeyPair, SignatureAlgorithm,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, ServerConfig, SignatureScheme,
    StreamOwned,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// LocalSend v2.1 multicast discovery address.
pub const MULTICAST_ADDR: std::net::Ipv4Addr = std::net::Ipv4Addr::new(224, 0, 0, 167);
/// LocalSend protocol version advertised in DTOs.
pub const PROTOCOL_VERSION: &str = "2.1";

/// Device metadata returned by `/info`, `/register`, and embedded in multicast announcements.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoDto {
    pub alias: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: String,
    #[serde(default)]
    pub download: bool,
}

/// UDP multicast discovery message (`224.0.0.167:53317`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MulticastDto {
    pub alias: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub fingerprint: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub download: bool,
    /// LocalSend v2 uses `announce`; v1/legacy peers may send `announcement`.
    #[serde(default)]
    pub announce: bool,
    #[serde(default, skip_serializing)]
    pub announcement: Option<bool>,
}

fn default_port() -> u16 {
    super::DEFAULT_PORT
}

fn default_protocol() -> String {
    "https".into()
}

impl MulticastDto {
    pub fn from_info(info: &InfoDto, announce: bool) -> Self {
        Self {
            alias: info.alias.clone(),
            version: info.version.clone(),
            device_model: info.device_model.clone(),
            device_type: info.device_type.clone(),
            fingerprint: info.fingerprint.clone(),
            port: info.port,
            protocol: info.protocol.clone(),
            download: info.download,
            announce,
            announcement: None,
        }
    }

    pub fn is_announce(&self) -> bool {
        self.announce || self.announcement.unwrap_or(false)
    }
}

/// Request body of `POST /api/localsend/v2/prepare-upload`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadRequest {
    #[serde(default)]
    pub info: serde_json::Value,
    pub files: BTreeMap<String, FileMetadata>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub id: String,
    pub file_name: String,
    pub size: u64,
    #[serde(default)]
    pub file_type: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Response body of `POST /api/localsend/v2/prepare-upload`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadResponse {
    pub session_id: String,
    pub files: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// TLS self-signed P-256 identity (no `ring`, pure Rust + Graviola provider)
// ---------------------------------------------------------------------------

struct P256RemoteKey {
    signing: SigningKey,
    pub_sec1: Vec<u8>,
}

impl RemoteKeyPair for P256RemoteKey {
    fn public_key(&self) -> &[u8] {
        &self.pub_sec1
    }

    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        use p256::ecdsa::signature::Signer;
        let sig: Signature = self.signing.sign(msg);
        let der: DerSignature = sig.to_der();
        Ok(der.as_bytes().to_vec())
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &rcgen::PKCS_ECDSA_P256_SHA256
    }
}

/// Persistent self-signed TLS certificate and its SHA-256 hex fingerprint.
pub struct TlsIdentity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: String,
}

impl TlsIdentity {
    /// Loads `cert.der` + `key.der` from `state_dir`, or generates and saves a new P-256 identity.
    pub fn load_or_create(state_dir: &Path, alias: &str) -> io::Result<Self> {
        let cert_path = state_dir.join("cert.der");
        let key_path = state_dir.join("key.der");
        if let (Ok(cert_der), Ok(key_der)) = (fs::read(&cert_path), fs::read(&key_path)) {
            if !cert_der.is_empty() && !key_der.is_empty() {
                let id = Self {
                    fingerprint: hex_lower(&Sha256::digest(&cert_der)),
                    cert_der,
                    key_der,
                };
                // Verify the stored keypair still loads cleanly before trusting it.
                if id.server_config().is_ok() {
                    return Ok(id);
                }
            }
        }
        let id = Self::generate(alias)?;
        fs::create_dir_all(state_dir)?;
        fs::write(&cert_path, &id.cert_der)?;
        fs::write(&key_path, &id.key_der)?;
        let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
        Ok(id)
    }

    pub fn generate(alias: &str) -> io::Result<Self> {
        let signing = SigningKey::random(&mut OsRng);
        let key_doc = signing
            .to_pkcs8_der()
            .map_err(|e| io::Error::other(format!("pkcs8 encode failed: {e}")))?;
        let pub_sec1 = signing
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        let remote = P256RemoteKey { signing, pub_sec1 };
        let key_pair = KeyPair::from_remote(Box::new(remote))
            .map_err(|e| io::Error::other(format!("rcgen keypair failed: {e}")))?;

        let mut params = CertificateParams::new(vec!["localhost".into(), "nibos.local".into()])
            .map_err(|e| io::Error::other(format!("rcgen params failed: {e}")))?;
        let mut serial = [0u8; 16];
        OsRng.fill_bytes(&mut serial);
        serial[0] &= 0x7F; // positive ASN.1 INTEGER
        params.serial_number = Some(rcgen::SerialNumber::from_slice(&serial));
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, alias);
        params.distinguished_name = dn;
        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| io::Error::other(format!("rcgen self-signed failed: {e}")))?;
        let cert_der = cert.der().to_vec();
        let fingerprint = hex_lower(&Sha256::digest(&cert_der));
        Ok(Self {
            cert_der,
            key_der: key_doc.as_bytes().to_vec(),
            fingerprint,
        })
    }

    pub fn server_config(&self) -> io::Result<Arc<ServerConfig>> {
        let provider = Arc::new(rustls_graviola::default_provider());
        let cert = CertificateDer::from(self.cert_der.clone());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()));
        let cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| io::Error::other(format!("rustls versions: {e}")))?
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .map_err(|e| io::Error::other(format!("rustls cert: {e}")))?;
        Ok(Arc::new(cfg))
    }
}

/// Generates a random URL-safe hex token (`bytes * 2` chars).
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    hex_lower(&buf)
}

pub fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

// ---------------------------------------------------------------------------
// Outbound peer `/register` (HTTPS accepting peer self-signed cert, or HTTP)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct AcceptSelfSigned;

impl ServerCertVerifier for AcceptSelfSigned {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls_graviola::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Sends `POST /api/localsend/v2/register` to a discovered peer so it lists us immediately.
pub fn register_with_peer(ip: IpAddr, port: u16, protocol: &str, our_info: &InfoDto) {
    let addr = SocketAddr::new(ip, port);
    let Ok(mut tcp) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return;
    };
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(2)));
    let Ok(body) = serde_json::to_vec(our_info) else {
        return;
    };
    let req = format!(
        "POST /api/localsend/v2/register HTTP/1.1\r\nHost: {ip}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if protocol.eq_ignore_ascii_case("http") {
        let _ = tcp.write_all(req.as_bytes());
        let _ = tcp.write_all(&body);
        let _ = tcp.flush();
        return;
    }
    let provider = Arc::new(rustls_graviola::default_provider());
    let Ok(cfg) = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map(|b| {
            b.dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptSelfSigned))
                .with_no_client_auth()
        })
    else {
        return;
    };
    let server_name = ServerName::from(ip);
    let Ok(conn) = ClientConnection::new(Arc::new(cfg), server_name) else {
        return;
    };
    let mut tls = StreamOwned::new(conn, tcp);
    let _ = tls.write_all(req.as_bytes());
    let _ = tls.write_all(&body);
    let _ = tls.flush();
    let mut sink = [0u8; 512];
    let _ = tls.read(&mut sink);
}

// ---------------------------------------------------------------------------
// Minimal HTTP/1.1 request parser & streaming body reader
// ---------------------------------------------------------------------------

/// Parsed HTTP/1.1 request head plus any body bytes already read alongside headers.
pub struct HttpRequestHead {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub content_length: Option<u64>,
    pub chunked: bool,
    pub expect_continue: bool,
    pub leftover: Vec<u8>,
}

/// Reads an HTTP/1.1 request line and headers (capped at 64 KiB).
pub fn read_http_head<S: Read + Write>(stream: &mut S) -> io::Result<HttpRequestHead> {
    const MAX_HEAD: usize = 64 * 1024;
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 2048];
    let header_end;
    loop {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof in http head",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_crlf2(&buf) {
            header_end = pos;
            break;
        }
        if buf.len() > MAX_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "http headers too large",
            ));
        }
    }
    let head_str = std::str::from_utf8(&buf[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-utf8 http headers"))?;
    let mut lines = head_str.split("\r\n");
    let req_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty request line"))?;
    let mut parts = req_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?
        .to_owned();
    let target = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing path"))?;
    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let path = raw_path.to_owned();
    let query = parse_query(raw_query);

    let mut content_length = None;
    let mut chunked = false;
    let mut expect_continue = false;
    for line in lines {
        let Some((name, val)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let val = val.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = val.parse::<u64>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && val.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        } else if name.eq_ignore_ascii_case("expect")
            && val.to_ascii_lowercase().contains("100-continue")
        {
            expect_continue = true;
        }
    }
    if expect_continue {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        stream.flush()?;
    }
    let leftover = buf[header_end + 4..].to_vec();
    Ok(HttpRequestHead {
        method,
        path,
        query,
        content_length,
        chunked,
        expect_continue,
        leftover,
    })
}

/// Reads a bounded HTTP body into memory (for small JSON payloads like `/prepare-upload`).
pub fn read_body_bytes<R: Read>(
    stream: &mut R,
    head: &HttpRequestHead,
    max_bytes: usize,
) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut sink = LimitedVec {
        buf: &mut out,
        max: max_bytes,
    };
    let _ = copy_http_body(stream, head, &mut sink, None, |_| {})?;
    Ok(out)
}

struct LimitedVec<'a> {
    buf: &'a mut Vec<u8>,
    max: usize,
}

impl Write for LimitedVec<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len() + data.len() > self.max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request body too large",
            ));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Streams an HTTP/1.1 request body directly into `file` with a fixed 64 KiB buffer, computing
/// SHA-256 on the fly and reporting byte progress.
pub fn stream_body_to_file<R: Read>(
    stream: &mut R,
    head: &HttpRequestHead,
    file: &mut File,
    max_bytes: u64,
    on_progress: impl FnMut(u64),
) -> io::Result<(u64, String)> {
    copy_http_body(stream, head, file, Some(max_bytes), on_progress)
}

fn copy_http_body<R: Read, W: Write>(
    stream: &mut R,
    head: &HttpRequestHead,
    dst: &mut W,
    max_bytes: Option<u64>,
    mut on_progress: impl FnMut(u64),
) -> io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut write_chunk = |data: &[u8]| -> io::Result<()> {
        if let Some(limit) = max_bytes {
            if total.saturating_add(data.len() as u64) > limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "upload exceeded declared size",
                ));
            }
        }
        dst.write_all(data)?;
        hasher.update(data);
        total += data.len() as u64;
        on_progress(total);
        Ok(())
    };

    let mut reader = PrefixedReader {
        prefix: &head.leftover,
        pos: 0,
        inner: stream,
    };

    if head.chunked {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let line = read_crlf_line(&mut reader)?;
            let size_hex = line.split(';').next().unwrap_or("").trim();
            let mut rem = u64::from_str_radix(size_hex, 16)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
            if rem == 0 {
                // Consume trailing CRLF (and optional trailers) until empty line.
                loop {
                    let trailer = read_crlf_line(&mut reader)?;
                    if trailer.is_empty() {
                        break;
                    }
                }
                break;
            }
            while rem > 0 {
                let take = (rem as usize).min(buf.len());
                let n = reader.read(&mut buf[..take])?;
                if n == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in chunk"));
                }
                write_chunk(&buf[..n])?;
                rem -= n as u64;
            }
            let end = read_crlf_line(&mut reader)?;
            if !end.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing chunk crlf",
                ));
            }
        }
    } else if let Some(mut rem) = head.content_length {
        let mut buf = vec![0u8; 64 * 1024];
        while rem > 0 {
            let take = (rem as usize).min(buf.len());
            let n = reader.read(&mut buf[..take])?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "eof before content-length",
                ));
            }
            write_chunk(&buf[..n])?;
            rem -= n as u64;
        }
    }
    dst.flush()?;
    Ok((total, hex_lower(&hasher.finalize())))
}

struct PrefixedReader<'a, R> {
    prefix: &'a [u8],
    pos: usize,
    inner: &'a mut R,
}

impl<R: Read> Read for PrefixedReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos < self.prefix.len() {
            let n = (self.prefix.len() - self.pos).min(buf.len());
            buf[..n].copy_from_slice(&self.prefix[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        self.inner.read(buf)
    }
}

fn read_crlf_line<R: Read>(r: &mut R) -> io::Result<String> {
    let mut line = Vec::with_capacity(32);
    let mut b = [0u8; 1];
    loop {
        if r.read(&mut b)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof in chunk line",
            ));
        }
        if b[0] == b'\n' {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return String::from_utf8(line)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-utf8 chunk line"));
        }
        line.push(b[0]);
        if line.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk line too long",
            ));
        }
    }
}

pub fn write_http_response<W: Write>(
    stream: &mut W,
    status: u16,
    reason: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> io::Result<()> {
    let ct = match content_type {
        Some(t) => format!("Content-Type: {t}\r\n"),
        None => String::new(),
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n{ct}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()
}

pub fn write_json_response<W: Write, T: Serialize>(
    stream: &mut W,
    status: u16,
    reason: &str,
    value: &T,
) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_http_response(stream, status, reason, Some("application/json"), &body)
}

pub fn write_error_response<W: Write>(
    stream: &mut W,
    status: u16,
    reason: &str,
    message: &str,
) -> io::Result<()> {
    #[derive(Serialize)]
    struct ErrBody<'a> {
        message: &'a str,
    }
    write_json_response(stream, status, reason, &ErrBody { message })
}

fn find_crlf2(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub fn parse_query(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(k), percent_decode(v));
    }
    map
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct MemStream {
        read: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl Read for MemStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read.read(buf)
        }
    }

    impl Write for MemStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn generates_valid_p256_tls_identity() {
        let id = TlsIdentity::generate("RG DS+").unwrap();
        assert_eq!(id.fingerprint.len(), 64);
        assert!(id.server_config().is_ok());
    }

    #[test]
    fn parses_http_query_and_chunked_body() {
        let raw = concat!(
            "POST /api/localsend/v2/upload?sessionId=s1&fileId=f%201&token=t1 HTTP/1.1\r\n",
            "Host: 127.0.0.1:53317\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Expect: 100-continue\r\n",
            "\r\n",
            "5\r\nhello\r\n",
            "6\r\n world\r\n",
            "0\r\n\r\n"
        );
        let mut s = MemStream {
            read: Cursor::new(raw.as_bytes().to_vec()),
            written: Vec::new(),
        };
        let head = read_http_head(&mut s).unwrap();
        assert_eq!(head.method, "POST");
        assert_eq!(head.path, "/api/localsend/v2/upload");
        assert_eq!(head.query.get("fileId").map(String::as_str), Some("f 1"));
        assert!(String::from_utf8_lossy(&s.written).contains("100 Continue"));
        let body = read_body_bytes(&mut s, &head, 1024).unwrap();
        assert_eq!(body, b"hello world");
    }
}

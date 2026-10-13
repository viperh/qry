//! Unit tests for `oracle.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::ConnectionConfig;

fn config() -> OracleConfig {
    OracleConfig {
        host: "db.invalid".into(),
        port: 1521,
        user: "alice".into(),
        password: "hunter2".into(),
        service: "ORCLPDB1".into(),
        schema: None,
        ssl_mode: SslMode::Prefer,
        wallet: None,
    }
}

fn tls() -> OracleConfig {
    OracleConfig {
        ssl_mode: SslMode::VerifyFull,
        ..config()
    }
}

/// The wallet tests need real files; each gets its own directory, removed
/// when the returned guard is dropped.
fn wallet_dir(pem: Option<&str>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(pem) = pem {
        fs::write(dir.path().join("ewallet.pem"), pem).unwrap();
    }
    let path = dir.path().display().to_string();
    (dir, path)
}

/// Accepts one connection on loopback, connects to it with `config`,
/// and answers with the first bytes the driver sent.
async fn first_bytes(config: OracleConfig) -> [u8; 5] {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        use std::io::Read;
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0; 5];
        stream.read_exact(&mut bytes).unwrap();
        bytes
    });
    let config = OracleConfig {
        host: "127.0.0.1".into(),
        port,
        ..config
    };
    // The server hangs up after reading, so this is always an error.
    assert!(Oracle::connect(config).await.is_err());
    server.join().unwrap()
}

#[test]
fn label_is_user_at_host_port_service() {
    assert_eq!(config().label(), "alice@db.invalid:1521/ORCLPDB1");
    assert_eq!(config().connect_string(), "//db.invalid:1521/ORCLPDB1");
}

#[test]
fn require_and_stricter_go_over_tcps() {
    for mode in SslMode::ALL {
        let config = OracleConfig {
            ssl_mode: mode,
            ..config()
        };
        let tls = matches!(
            mode,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull
        );
        assert_eq!(config.uses_tls(), tls, "{mode}");
        let expected = if tls { "tcps://" } else { "//" };
        assert!(config.connect_string().starts_with(expected), "{mode}");
    }
    let descriptor = tls().driver_config().unwrap().get_connect_descriptor();
    assert!(
        descriptor.to_lowercase().contains("protocol=tcps"),
        "{descriptor}"
    );
}

#[test]
fn the_driver_accepts_the_connect_string() {
    let parsed = config().driver_config().unwrap();
    assert_eq!(parsed.user(), Some("alice"));
    let descriptor = parsed.get_connect_descriptor();
    assert!(descriptor.contains("db.invalid"), "{descriptor}");
    assert!(descriptor.contains("1521"), "{descriptor}");
    assert!(descriptor.contains("ORCLPDB1"), "{descriptor}");
    assert!(!descriptor.contains("hunter2"), "{descriptor}");
}

#[test]
fn debug_output_never_contains_the_password() {
    let shown = format!("{:?}", ConnectionConfig::Oracle(config()));
    assert_eq!(shown, "Oracle(alice@db.invalid:1521/ORCLPDB1)");
    assert!(!shown.contains("hunter2"));
}

#[test]
fn identifiers_are_quoted_for_the_session_schema() {
    assert_eq!(quote_ident("hr"), "\"hr\"");
    assert_eq!(quote_ident("my\"schema"), "\"my\"\"schema\"");
}

#[test]
fn json_columns_render_as_compact_json() {
    let object = JsonValue::JsonObject(HashMap::from([
        (
            "name".to_string(),
            JsonValue::String("a \"b\"\n\u{1}".into()),
        ),
        (
            "tags".to_string(),
            JsonValue::JsonArray(vec![
                JsonValue::Boolean(true),
                JsonValue::Null,
                JsonValue::BinaryDouble(1.5),
            ]),
        ),
        ("id".to_string(), JsonValue::Raw(vec![0xde, 0xad])),
    ]));
    assert_eq!(
        json(&object),
        r#"{"id":"dead","name":"a \"b\"\n\u0001","tags":[true,null,1.5]}"#
    );
}

#[tokio::test]
async fn a_wallet_without_tls_is_refused_before_connecting() {
    let config = OracleConfig {
        wallet: Some("/etc/oracle/wallet".into()),
        ..config()
    };
    // `db.invalid` never resolves, so an error here could only come
    // from the wallet check, which runs first.
    let message = match Oracle::connect(config).await {
        Ok(_) => panic!("a wallet over plain TCP must not connect"),
        Err(err) => err.to_string(),
    };
    assert!(message.contains("only used over TLS"), "{message}");
    assert!(message.contains("/etc/oracle/wallet"), "{message}");
}

#[tokio::test]
async fn a_host_no_certificate_can_name_is_refused_before_connecting() {
    let config = OracleConfig {
        host: "not a host".into(),
        ..tls()
    };
    let message = Oracle::connect(config).await.err().unwrap().to_string();
    assert!(message.contains("not a host name"), "{message}");
}

#[tokio::test]
async fn tls_is_negotiated_before_anything_else_is_sent() {
    // A TLS record starts 0x16 0x03: a handshake, then the version.
    let bytes = first_bytes(tls()).await;
    assert_eq!(bytes[..2], [0x16, 0x03], "{bytes:02x?}");

    // Without TLS the first thing on the wire is a TNS CONNECT packet,
    // type 1 in the fifth byte, and never a TLS record.
    let bytes = first_bytes(config()).await;
    assert_eq!(bytes[4], 1, "{bytes:02x?}");
    assert_ne!(bytes[0], 0x16, "{bytes:02x?}");
}

/// A TLS server on loopback with a certificate for 127.0.0.1, signed by
/// a test CA no system trusts. `testdata/ca.pem` is that CA; its key was
/// thrown away after signing. Answers with the first bytes read inside
/// the TLS session, or the handshake error.
fn tls_server() -> (u16, std::thread::JoinHandle<Result<[u8; 5], String>>) {
    use std::io::Read;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

    let certs = CertificateDer::pem_slice_iter(include_bytes!("../testdata/server.pem"))
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("../testdata/server.key")).unwrap();
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut conn = rustls::ServerConnection::new(Arc::new(tls)).unwrap();
        let mut session = rustls::Stream::new(&mut conn, &mut stream);
        let mut bytes = [0; 5];
        session.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    });
    (port, server)
}

#[tokio::test]
async fn an_untrusted_certificate_is_refused() {
    let (port, server) = tls_server();
    let config = OracleConfig {
        host: "127.0.0.1".into(),
        port,
        ..tls()
    };
    let message = Oracle::connect(config).await.err().unwrap().to_string();
    assert!(message.to_lowercase().contains("certificate"), "{message}");
    assert!(
        server.join().unwrap().is_err(),
        "nothing may cross an untrusted session"
    );
}

#[tokio::test]
async fn a_wallet_can_add_the_ca_to_trust() {
    let (_guard, dir) = wallet_dir(Some(include_str!("../testdata/ca.pem")));
    let (port, server) = tls_server();
    let config = OracleConfig {
        host: "127.0.0.1".into(),
        port,
        wallet: Some(dir),
        ..tls()
    };
    // The server hangs up once it has read, so connecting still fails,
    // but only after the handshake: the TNS CONNECT came through TLS.
    assert!(Oracle::connect(config).await.is_err());
    let bytes = server.join().unwrap().unwrap();
    assert_eq!(bytes[4], 1, "{bytes:02x?}");
}

#[tokio::test]
async fn a_missing_wallet_file_says_where_it_looked() {
    let (_guard, dir) = wallet_dir(None);
    let config = OracleConfig {
        wallet: Some(dir.clone()),
        host: "127.0.0.1".into(),
        port: 1,
        ..tls()
    };
    let message = Oracle::connect(config).await.err().unwrap().to_string();
    assert!(message.contains("ewallet.pem"), "{message}");
    assert!(message.contains(&dir), "{message}");
}

#[tokio::test]
async fn an_encrypted_wallet_key_is_explained() {
    let pem = "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n";
    let (_guard, dir) = wallet_dir(Some(pem));
    let config = OracleConfig {
        wallet: Some(dir),
        host: "127.0.0.1".into(),
        port: 1,
        ..tls()
    };
    let message = Oracle::connect(config).await.err().unwrap().to_string();
    assert!(message.contains("encrypted"), "{message}");
    assert!(message.contains("wallet password"), "{message}");
}

/// Nothing listens on port 1 on loopback, so the connection is refused
/// at once rather than waiting for a timeout. It must be an `Err`.
#[tokio::test]
async fn an_unreachable_server_is_an_error_not_a_panic() {
    let config = OracleConfig {
        host: "127.0.0.1".into(),
        port: 1,
        ..config()
    };
    assert!(Oracle::connect(config).await.is_err());
}

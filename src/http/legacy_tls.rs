//! `--legacy-server-connect`: upstream's `legacy_ssl_support` sets OpenSSL's
//! `DEFAULT` ciphers (and `OP_LEGACY_SERVER_CONNECT`, which Rustls needs
//! no flag for: it never requires RFC 5746 secure renegotiation). Rustls has
//! no CBC cipher suites, so this module adds the ECDHE TLS 1.2 `AES_CBC_SHA`
//! and `AES_128_CBC_SHA256` suites (RFC 5246 MAC-then-encrypt with explicit
//! IVs) after the default ones.
//!
//! Rustls splits a TLS 1.2 key block as two `enc_key_len` keys, two
//! `fixed_iv_len` IVs and extra bytes. The CBC key block is two MAC keys and
//! two encryption keys (TLS 1.2 has no fixed CBC IVs), so the MAC key takes
//! the "key" slot and the encryption key the "IV" slot. The 32-byte key
//! slot excludes SHA-384 MACs; Rustls has no static RSA key exchange.
//! Padding and MAC checks are not constant-time (Lucky Thirteen): an opt-in
//! for servers that offer nothing better, like upstream's option.
use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use rustls::crypto::cipher::{
    AeadKey, InboundOpaqueMessage, InboundPlainMessage, KeyBlockShape, MessageDecrypter,
    MessageEncrypter, OutboundOpaqueMessage, OutboundPlainMessage, PrefixedPayload,
    Tls12AeadAlgorithm, UnsupportedOperationError, make_tls12_aad,
};
use rustls::crypto::{CipherSuiteCommon, CryptoProvider, KeyExchangeAlgorithm};
use rustls::{CipherSuite, ConnectionTrafficSecrets, SupportedCipherSuite, Tls12CipherSuite};
use std::sync::{Arc, LazyLock};

const BLOCK: usize = 16;

#[derive(Clone, Copy)]
enum Mac {
    Sha1,
    Sha256,
}

impl Mac {
    fn algorithm(self) -> ring::hmac::Algorithm {
        match self {
            Self::Sha1 => ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            Self::Sha256 => ring::hmac::HMAC_SHA256,
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
        }
    }
}

/// An AES-CBC-HMAC record protection (the suite's "AEAD" for Rustls).
struct Cbc {
    key_len: usize,
    mac: Mac,
}

enum Aes {
    A128(Box<aes::Aes128>),
    A256(Box<aes::Aes256>),
}

impl Aes {
    fn new(key: &[u8]) -> Self {
        match key.len() {
            16 => Self::A128(Box::new(aes::Aes128::new(
                &Array::try_from(key).expect("16 bytes"),
            ))),
            _ => Self::A256(Box::new(aes::Aes256::new(
                &Array::try_from(key).expect("32 bytes"),
            ))),
        }
    }

    fn encrypt(&self, block: &mut [u8]) {
        let block: &mut Array<u8, _> = block.try_into().expect("one block");
        match self {
            Self::A128(cipher) => cipher.encrypt_block(block),
            Self::A256(cipher) => cipher.encrypt_block(block),
        }
    }

    fn decrypt(&self, block: &mut [u8]) {
        let block: &mut Array<u8, _> = block.try_into().expect("one block");
        match self {
            Self::A128(cipher) => cipher.decrypt_block(block),
            Self::A256(cipher) => cipher.decrypt_block(block),
        }
    }
}

struct Protection {
    aes: Aes,
    mac: ring::hmac::Key,
    mac_len: usize,
}

impl Protection {
    fn new(cbc: &Cbc, mac_key: &AeadKey, enc_key: &[u8]) -> Self {
        Self {
            aes: Aes::new(enc_key),
            mac: ring::hmac::Key::new(cbc.mac.algorithm(), mac_key.as_ref()),
            mac_len: cbc.mac.len(),
        }
    }

    /// `HMAC(seq_num + type + version + length + fragment)`.
    fn tag(&self, aad: &[u8], plaintext: &[u8]) -> ring::hmac::Tag {
        let mut context = ring::hmac::Context::with_key(&self.mac);
        context.update(aad);
        context.update(plaintext);
        context.sign()
    }
}

impl MessageEncrypter for Protection {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, rustls::Error> {
        let mut data = msg.payload.to_vec();
        let aad = make_tls12_aad(seq, msg.typ, msg.version, data.len());
        let tag = self.tag(&aad, &data);
        data.extend_from_slice(tag.as_ref());
        // Padding of `pad` bytes valued `pad`, plus the length byte.
        let pad = BLOCK - 1 - data.len() % BLOCK;
        data.extend(std::iter::repeat_n(pad as u8, pad + 1));
        let mut iv = [0u8; BLOCK];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut iv)
            .map_err(|_| rustls::Error::FailedToGetRandomBytes)?;
        let mut previous = iv;
        for block in data.chunks_mut(BLOCK) {
            for (byte, chained) in block.iter_mut().zip(previous) {
                *byte ^= chained;
            }
            self.aes.encrypt(block);
            previous.copy_from_slice(block);
        }
        let mut payload = PrefixedPayload::with_capacity(BLOCK + data.len());
        payload.extend_from_slice(&iv);
        payload.extend_from_slice(&data);
        Ok(OutboundOpaqueMessage::new(msg.typ, msg.version, payload))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        let data = payload_len + self.mac_len + 1;
        BLOCK + data.div_ceil(BLOCK) * BLOCK
    }
}

impl MessageDecrypter for Protection {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, rustls::Error> {
        let payload = &mut msg.payload;
        let length = payload.len();
        if length < BLOCK + (self.mac_len + 1).div_ceil(BLOCK) * BLOCK
            || !length.is_multiple_of(BLOCK)
        {
            return Err(rustls::Error::DecryptError);
        }
        let mut previous = [0u8; BLOCK];
        previous.copy_from_slice(&payload[..BLOCK]);
        for block in payload[BLOCK..].chunks_mut(BLOCK) {
            let mut ciphertext = [0u8; BLOCK];
            ciphertext.copy_from_slice(block);
            self.aes.decrypt(block);
            for (byte, chained) in block.iter_mut().zip(previous) {
                *byte ^= chained;
            }
            previous = ciphertext;
        }
        let data = &payload[BLOCK..];
        let pad = usize::from(data[data.len() - 1]);
        if pad + 1 + self.mac_len > data.len()
            || data[data.len() - 1 - pad..]
                .iter()
                .any(|&byte| usize::from(byte) != pad)
        {
            return Err(rustls::Error::DecryptError);
        }
        let plaintext_len = data.len() - pad - 1 - self.mac_len;
        let aad = make_tls12_aad(seq, msg.typ, msg.version, plaintext_len);
        let mut context = ring::hmac::Context::with_key(&self.mac);
        context.update(&aad);
        context.update(&data[..plaintext_len]);
        let expected = context.sign();
        let received = &data[plaintext_len..plaintext_len + self.mac_len];
        // `ring::hmac::verify` recomputes; compare the tags in constant time.
        let equal = expected
            .as_ref()
            .iter()
            .zip(received)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0;
        if !equal {
            return Err(rustls::Error::DecryptError);
        }
        Ok(msg.into_plain_message_range(BLOCK..BLOCK + plaintext_len))
    }
}

impl Tls12AeadAlgorithm for Cbc {
    fn encrypter(&self, key: AeadKey, iv: &[u8], _: &[u8]) -> Box<dyn MessageEncrypter> {
        Box::new(Protection::new(self, &key, iv))
    }

    fn decrypter(&self, key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
        Box::new(Protection::new(self, &key, iv))
    }

    fn key_block_shape(&self) -> KeyBlockShape {
        KeyBlockShape {
            enc_key_len: self.mac.len(),
            fixed_iv_len: self.key_len,
            explicit_nonce_len: 0,
        }
    }

    fn extract_keys(
        &self,
        _: AeadKey,
        _: &[u8],
        _: &[u8],
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Err(UnsupportedOperationError)
    }
}

/// The default TLS 1.2 suite whose PRF, hash and signature schemes a CBC
/// suite with the same key exchange (RSA or ECDSA) shares.
fn template(ecdsa: bool) -> &'static Tls12CipherSuite {
    let suite = if ecdsa {
        rustls::crypto::ring::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
    } else {
        rustls::crypto::ring::cipher_suite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
    };
    match suite {
        SupportedCipherSuite::Tls12(suite) => suite,
        SupportedCipherSuite::Tls13(_) => unreachable!("a TLS 1.2 suite"),
    }
}

fn suite(id: CipherSuite, ecdsa: bool, key_len: usize, mac: Mac) -> SupportedCipherSuite {
    let template = template(ecdsa);
    let aead: &'static Cbc = Box::leak(Box::new(Cbc { key_len, mac }));
    SupportedCipherSuite::Tls12(Box::leak(Box::new(Tls12CipherSuite {
        common: CipherSuiteCommon {
            suite: id,
            // TLS 1.2's PRF is SHA-256 for these suites.
            hash_provider: template.common.hash_provider,
            confidentiality_limit: u64::MAX,
        },
        kx: KeyExchangeAlgorithm::ECDHE,
        sign: template.sign,
        aead_alg: aead,
        prf_provider: template.prf_provider,
    })))
}

/// The CBC suites, in OpenSSL's `DEFAULT` preference among them.
pub(crate) static SUITES: LazyLock<Vec<SupportedCipherSuite>> = LazyLock::new(|| {
    vec![
        suite(
            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA,
            true,
            32,
            Mac::Sha1,
        ),
        suite(
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA,
            false,
            32,
            Mac::Sha1,
        ),
        suite(
            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA256,
            true,
            16,
            Mac::Sha256,
        ),
        suite(
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256,
            false,
            16,
            Mac::Sha256,
        ),
        suite(
            CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA,
            true,
            16,
            Mac::Sha1,
        ),
        suite(
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA,
            false,
            16,
            Mac::Sha1,
        ),
    ]
});

/// Rustls' defaults followed by the CBC suites.
pub(crate) fn provider() -> Arc<CryptoProvider> {
    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites.extend(SUITES.iter().copied());
    Arc::new(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use std::io::{Read, Write};

    const CA: &[u8] = include_bytes!("../../tests/fixtures/proxies/tls/ca.der");
    const CERT: &[u8] = include_bytes!("../../tests/fixtures/proxies/tls/server.der");
    const KEY: &[u8] = include_bytes!("../../tests/fixtures/proxies/tls/key.der");

    /// A Rustls server offering only `suite`; the client uses [`provider`].
    fn exchange(suite: SupportedCipherSuite) -> (Vec<u8>, CipherSuite) {
        let mut server_provider = rustls::crypto::ring::default_provider();
        server_provider.cipher_suites = vec![suite];
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(server_provider))
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERT.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
            )
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(CA.to_vec())).unwrap();
        let client = rustls::ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let connection = rustls::ServerConnection::new(Arc::new(server)).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, stream);
            let mut request = [0u8; 5];
            tls.read_exact(&mut request).unwrap();
            // Several records, one longer than a block multiple.
            tls.write_all(&request).unwrap();
            tls.write_all(&[7u8; 4000]).unwrap();
            tls.conn.send_close_notify();
            tls.flush().unwrap();
        });
        let connection = rustls::ClientConnection::new(
            Arc::new(client),
            "media.example.test".try_into().unwrap(),
        )
        .unwrap();
        let mut tls =
            rustls::StreamOwned::new(connection, std::net::TcpStream::connect(address).unwrap());
        tls.write_all(b"hello").unwrap();
        let mut received = Vec::new();
        let _ = tls.read_to_end(&mut received);
        let negotiated = tls.conn.negotiated_cipher_suite().unwrap().suite();
        server.join().unwrap();
        (received, negotiated)
    }

    #[test]
    fn rsa_cbc_suites_round_trip() {
        for suite in SUITES.iter().copied() {
            let id = suite.suite();
            if format!("{id:?}").contains("ECDSA") {
                continue; // The fixture key is RSA.
            }
            let (received, negotiated) = exchange(suite);
            assert_eq!(negotiated, id);
            assert_eq!(&received[..5], b"hello", "{id:?}");
            assert_eq!(received.len(), 4005, "{id:?}");
            assert!(received[5..].iter().all(|&byte| byte == 7), "{id:?}");
        }
    }

    #[test]
    fn records_have_explicit_iv_mac_and_padding() {
        let cbc = Cbc {
            key_len: 16,
            mac: Mac::Sha1,
        };
        let mut protection = Protection::new(&cbc, &AeadKey::from([1u8; 32]), &[2u8; 16]);
        // 16-byte IV, then 5 + 20 + 1 bytes padded to 32.
        assert_eq!(protection.encrypted_payload_len(5), 48);
        let message = OutboundPlainMessage {
            typ: rustls::ContentType::ApplicationData,
            version: rustls::ProtocolVersion::TLSv1_2,
            payload: b"hello".as_slice().into(),
        };
        let encrypted = protection.encrypt(message, 9).unwrap().encode();
        let mut payload = encrypted[5..].to_vec();
        assert_eq!(payload.len(), 48);
        let inbound = InboundOpaqueMessage::new(
            rustls::ContentType::ApplicationData,
            rustls::ProtocolVersion::TLSv1_2,
            &mut payload,
        );
        assert_eq!(protection.decrypt(inbound, 9).unwrap().payload, b"hello");
        // A wrong sequence number fails the MAC.
        let mut payload = encrypted[5..].to_vec();
        let inbound = InboundOpaqueMessage::new(
            rustls::ContentType::ApplicationData,
            rustls::ProtocolVersion::TLSv1_2,
            &mut payload,
        );
        assert!(protection.decrypt(inbound, 10).is_err());
    }
}

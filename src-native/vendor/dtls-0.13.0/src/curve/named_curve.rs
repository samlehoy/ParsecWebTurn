use p256::elliptic_curve::Generate;
use rand_core::OsRng; // requires 'getrandom' feature

use crate::error::*;

// https://www.iana.org/assignments/tls-parameters/tls-parameters.xml#tls-parameters-8
#[repr(u16)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum NamedCurve {
    Unsupported = 0x0000,
    P256 = 0x0017,
    P384 = 0x0018,
    X25519 = 0x001d,
}

impl From<u16> for NamedCurve {
    fn from(val: u16) -> Self {
        match val {
            0x0017 => NamedCurve::P256,
            0x0018 => NamedCurve::P384,
            0x001d => NamedCurve::X25519,
            _ => NamedCurve::Unsupported,
        }
    }
}

pub(crate) enum NamedCurvePrivateKey {
    EphemeralSecretP256(p256::ecdh::EphemeralSecret),
    EphemeralSecretP384(p384::ecdh::EphemeralSecret),
    StaticSecretX25519(x25519_dalek::StaticSecret),
}

pub struct NamedCurveKeypair {
    pub(crate) curve: NamedCurve,
    pub(crate) public_key: Vec<u8>,
    pub(crate) private_key: NamedCurvePrivateKey,
}

fn elliptic_curve_keypair(curve: NamedCurve) -> Result<NamedCurveKeypair> {
    let (public_key, private_key) = match curve {
        NamedCurve::P256 => {
            let secret_key = p256::ecdh::EphemeralSecret::try_generate()
                .map_err(|error| Error::Other(format!("P-256 key generation: {error}")))?;
            let public_key = p256::Sec1Point::from(secret_key.public_key());
            (
                public_key.as_bytes().to_vec(),
                NamedCurvePrivateKey::EphemeralSecretP256(secret_key),
            )
        }
        NamedCurve::P384 => {
            let secret_key = p384::ecdh::EphemeralSecret::try_generate()
                .map_err(|error| Error::Other(format!("P-384 key generation: {error}")))?;
            let public_key = p384::Sec1Point::from(secret_key.public_key());
            (
                public_key.as_bytes().to_vec(),
                NamedCurvePrivateKey::EphemeralSecretP384(secret_key),
            )
        }
        NamedCurve::X25519 => {
            let secret_key = x25519_dalek::StaticSecret::random_from_rng(OsRng);
            let public_key = x25519_dalek::PublicKey::from(&secret_key);
            (
                public_key.as_bytes().to_vec(),
                NamedCurvePrivateKey::StaticSecretX25519(secret_key),
            )
        }
        _ => return Err(Error::ErrInvalidNamedCurve),
    };

    Ok(NamedCurveKeypair {
        curve,
        public_key,
        private_key,
    })
}

impl NamedCurve {
    pub fn generate_keypair(&self) -> Result<NamedCurveKeypair> {
        match *self {
            NamedCurve::X25519 => elliptic_curve_keypair(NamedCurve::X25519),
            NamedCurve::P256 => elliptic_curve_keypair(NamedCurve::P256),
            NamedCurve::P384 => elliptic_curve_keypair(NamedCurve::P384),
            _ => Err(Error::ErrInvalidNamedCurve),
        }
    }
}

#[cfg(test)]
mod evoparseq_tests {
    use super::*;
    use crate::prf::prf_pre_master_secret;
    use ring::{agreement, rand::SystemRandom};

    #[test]
    fn ecdh_wire_points_and_shared_secrets_match_ring_for_all_curves() {
        for (curve, algorithm, wire_len) in [
            (NamedCurve::P256, &agreement::ECDH_P256, 65),
            (NamedCurve::P384, &agreement::ECDH_P384, 97),
            (NamedCurve::X25519, &agreement::X25519, 32),
        ] {
            let local = curve.generate_keypair().unwrap();
            assert_eq!(local.public_key.len(), wire_len, "{curve:?}");
            if curve != NamedCurve::X25519 {
                assert_eq!(local.public_key[0], 4, "uncompressed TLS point");
            }
            let peer =
                agreement::EphemeralPrivateKey::generate(algorithm, &SystemRandom::new()).unwrap();
            let peer_public = peer.compute_public_key().unwrap();
            let secret =
                prf_pre_master_secret(peer_public.as_ref(), &local.private_key, curve).unwrap();
            let peer_secret = agreement::agree_ephemeral(
                peer,
                &agreement::UnparsedPublicKey::new(algorithm, &local.public_key),
                |secret| secret.to_vec(),
            )
            .unwrap();
            assert_eq!(secret, peer_secret, "{curve:?}");
        }
    }

    #[test]
    fn upgraded_curves_reject_invalid_peer_points() {
        for (curve, wire_len) in [(NamedCurve::P256, 65), (NamedCurve::P384, 97)] {
            let local = curve.generate_keypair().unwrap();
            assert!(prf_pre_master_secret(&[], &local.private_key, curve).is_err());
            let mut invalid = vec![0; wire_len];
            invalid[0] = 4; // SEC1 uncompressed marker, but (0, 0) is not on either curve.
            assert!(prf_pre_master_secret(&invalid, &local.private_key, curve).is_err());
        }
    }
}

use crate::types::{EncryptedTx, Transaction};
use sha2::{Digest, Sha256};

const PRIME: i128 = 2_147_483_647; // Mersenne prime 2^31 - 1

#[derive(Clone, Debug)]
pub struct ThresholdScheme {
    pub threshold: usize,
    pub total_validators: usize,
    pub secret: i128,
    pub polynomial: Vec<i128>,
}

impl ThresholdScheme {
    pub fn new(threshold: usize, total_validators: usize, secret_seed: u64) -> Self {
        assert!(threshold <= total_validators);
        let secret = (secret_seed as i128 % (PRIME - 1)) + 1;
        let mut polynomial = vec![secret];
        for i in 1..threshold {
            let coeff = ((secret_seed.wrapping_mul(i as u64 * 31 + 7)) as i128 % (PRIME - 1)) + 1;
            polynomial.push(coeff);
        }

        ThresholdScheme {
            threshold,
            total_validators,
            secret,
            polynomial,
        }
    }

    pub fn eval_polynomial(&self, x: i128) -> i128 {
        let mut result = 0i128;
        let mut x_pow = 1i128;
        for coeff in &self.polynomial {
            result = (result + (coeff * x_pow)) % PRIME;
            x_pow = (x_pow * x) % PRIME;
        }
        (result + PRIME) % PRIME
    }

    pub fn get_share(&self, validator_idx: usize) -> (i128, i128) {
        let x = (validator_idx + 1) as i128;
        let y = self.eval_polynomial(x);
        (x, y)
    }

    pub fn derive_key(secret: i128) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(secret.to_be_bytes());
        let hash = hasher.finalize();
        let mut key = [0u8; 32];
        key.copy_from_slice(&hash);
        key
    }

    pub fn encrypt(&self, tx: &Transaction) -> EncryptedTx {
        let key = Self::derive_key(self.secret);
        let serialized = serde_json::to_vec(tx).expect("serialize tx");
        let mut ciphertext = Vec::with_capacity(serialized.len());
        for (i, byte) in serialized.iter().enumerate() {
            ciphertext.push(byte ^ key[i % 32]);
        }

        let mut ephemeral = [0u8; 32];
        ephemeral[..8].copy_from_slice(&tx.id.to_be_bytes());

        EncryptedTx {
            id: tx.id,
            sender: tx.sender,
            ciphertext,
            ephemeral_key: ephemeral,
        }
    }

    pub fn reconstruct_secret(shares: &[(i128, i128)]) -> Result<i128, &'static str> {
        if shares.is_empty() {
            return Err("no shares");
        }
        let k = shares.len();
        let mut secret = 0i128;

        for i in 0..k {
            let (xi, yi) = shares[i];
            let mut num = 1i128;
            let mut den = 1i128;

            for j in 0..k {
                if i != j {
                    let (xj, _) = shares[j];
                    num = (num * (-xj)) % PRIME;
                    den = (den * (xi - xj)) % PRIME;
                }
            }

            let den_inv = mod_inverse((den + PRIME) % PRIME, PRIME);
            let term = (yi * num) % PRIME;
            let lagrange = (term * den_inv) % PRIME;
            secret = (secret + lagrange) % PRIME;
        }

        Ok((secret + PRIME) % PRIME)
    }

    pub fn decrypt_with_shares(shares: &[(i128, i128)], enc_tx: &EncryptedTx) -> Result<Transaction, &'static str> {
        let recovered_secret = Self::reconstruct_secret(shares)?;
        let key = Self::derive_key(recovered_secret);
        let mut plaintext = Vec::with_capacity(enc_tx.ciphertext.len());
        for (i, byte) in enc_tx.ciphertext.iter().enumerate() {
            plaintext.push(byte ^ key[i % 32]);
        }

        serde_json::from_slice(&plaintext).map_err(|_| "decryption/deserialize failed")
    }
}

fn mod_inverse(a: i128, m: i128) -> i128 {
    let (mut mn, mut xy) = ((m, a), (0i128, 1i128));
    while mn.1 != 0 {
        xy = (xy.1, xy.0 - (mn.0 / mn.1) * xy.1);
        mn = (mn.1, mn.0 % mn.1);
    }
    while xy.0 < 0 {
        xy.0 += m;
    }
    xy.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Address, TxPayload};

    #[test]
    fn test_threshold_crypto() {
        let scheme = ThresholdScheme::new(3, 5, 1337);
        let tx = Transaction {
            id: 1,
            sender: Address::new(42),
            nonce: 0,
            payload: TxPayload::Transfer {
                to: Address::new(99),
                amount: 1000,
            },
            gas_limit: 21000,
        };

        let enc_tx = scheme.encrypt(&tx);
        assert_ne!(enc_tx.ciphertext, serde_json::to_vec(&tx).unwrap());

        // 3 shares are enough
        let shares = vec![scheme.get_share(0), scheme.get_share(2), scheme.get_share(4)];
        let decrypted = ThresholdScheme::decrypt_with_shares(&shares, &enc_tx).unwrap();
        assert_eq!(decrypted, tx);
    }
}

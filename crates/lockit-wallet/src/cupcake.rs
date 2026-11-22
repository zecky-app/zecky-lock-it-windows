//! Cupcake's PCZT layout (Lock it Mac's CupcakePczt, 22 Nov 2026).
//!
//! Cupcake (Cake Wallet's offline signer, built on Zkool) reads and writes
//! PCZTs with the Zcash Shielded Assets branch of the pczt crate. Its layout
//! is the standard one (pczt 0.10, which Lock it, Zashi and Keystone use)
//! plus empty fields for assets:
//!  - every Orchard/Ironwood spend: `rseed_split_note` after `rseed`, and
//!    `asset` at the end;
//!  - every Orchard/Ironwood output: `asset` at the end;
//!  - the v2 encoding: an `issue` bundle at the very end.
//!
//! Global, transparent and Sapling parts are the same byte for byte.
//!
//! The PCZT is postcard: no field names, so this walks the whole layout and
//! copies it, adding the empty fields on the way to Cupcake and taking them
//! out of its signed reply. A filled-in asset or issue field means a ZSA
//! payment, which Lock it does not make, so it is refused rather than
//! dropped. Checked with the Mac's vectors (made with pczt 0.10.0 and the
//! zcash-shielded-assets librustzcash fb3b4de, which Cupcake 1.2.0 pins).

use crate::{Error, Result};

/// Lock it's PCZT, in the layout Cupcake reads.
pub fn to_cupcake(pczt: &[u8]) -> Result<Vec<u8>> {
    Walker::new(pczt, true).run()
}

/// Cupcake's signed PCZT, back in the standard layout.
pub fn from_cupcake(pczt: &[u8]) -> Result<Vec<u8>> {
    Walker::new(pczt, false).run()
}

fn fail(what: &str) -> Error {
    Error::Other(format!("the PCZT {what}"))
}

struct Walker<'a> {
    input: &'a [u8],
    to_cupcake: bool,
    at: usize,
    out: Vec<u8>,
}

impl<'a> Walker<'a> {
    fn new(input: &'a [u8], to_cupcake: bool) -> Self {
        Self { input, to_cupcake, at: 0, out: Vec::with_capacity(input.len() + 256) }
    }

    fn run(mut self) -> Result<Vec<u8>> {
        if self.input.len() < 8 || &self.input[..4] != b"PCZT" {
            return Err(fail("is not a PCZT"));
        }
        let version = u32::from_le_bytes(self.input[4..8].try_into().expect("4 bytes"));
        self.raw(8)?;
        match version {
            1 => {
                self.global()?;
                self.transparent()?;
                self.sapling(false)?;
                self.orchard(false)?;
            }
            2 => {
                self.global()?;
                self.option(|w| w.transparent())?;
                self.option(|w| w.sapling(true))?;
                self.option(|w| w.orchard(true))?; // Orchard
                self.option(|w| w.orchard(true))?; // Ironwood
                self.added("an asset issuance")?; // issue
            }
            v => return Err(fail(&format!("has an unknown version ({v})"))),
        }
        if self.at != self.input.len() {
            return Err(fail("has bytes after its end"));
        }
        Ok(self.out)
    }

    // ---- postcard ----

    fn raw(&mut self, n: usize) -> Result<()> {
        if self.input.len() - self.at < n {
            return Err(fail("ends too early"));
        }
        self.out.extend_from_slice(&self.input[self.at..self.at + n]);
        self.at += n;
        Ok(())
    }

    /// Copies a varint and returns it. `max` bytes: 5 for u32, 10 for u64, 19 for i128.
    fn varint(&mut self, max: usize) -> Result<u64> {
        let mut value: u64 = 0;
        for i in 0..max {
            let byte = *self.input.get(self.at).ok_or_else(|| fail("ends too early"))?;
            self.at += 1;
            self.out.push(byte);
            if i < 10 {
                value |= ((byte & 0x7f) as u64).checked_shl(7 * i as u32).unwrap_or(0);
            }
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(fail("has a number that is too long"))
    }

    fn length(&mut self) -> Result<usize> {
        let n = self.varint(10)?;
        // Every item is at least a byte.
        if n > (self.input.len() - self.at) as u64 {
            return Err(fail("ends too early"));
        }
        Ok(n as usize)
    }

    fn bytes(&mut self) -> Result<()> {
        let n = self.length()?;
        self.raw(n)
    }

    fn option(&mut self, body: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let tag = *self.input.get(self.at).ok_or_else(|| fail("ends too early"))?;
        if tag > 1 {
            return Err(fail("is not a PCZT"));
        }
        self.raw(1)?;
        if tag == 1 {
            body(self)?;
        }
        Ok(())
    }

    fn list(&mut self, mut body: impl FnMut(&mut Self) -> Result<()>) -> Result<()> {
        for _ in 0..self.length()? {
            body(self)?;
        }
        Ok(())
    }

    /// A field only Cupcake's layout has: written as None for Cupcake, and
    /// must be None when read back.
    fn added(&mut self, what: &str) -> Result<()> {
        if self.to_cupcake {
            self.out.push(0);
            return Ok(());
        }
        let tag = *self.input.get(self.at).ok_or_else(|| fail("ends too early"))?;
        if tag != 0 {
            return Err(Error::Other(format!("Cupcake signed something with {what}, which Lock it does not send. Nothing was sent.")));
        }
        self.at += 1;
        Ok(())
    }

    fn opt32(&mut self) -> Result<()> {
        self.option(|w| w.raw(32))
    }
    fn u32(&mut self) -> Result<()> {
        self.varint(5).map(drop)
    }
    fn u64(&mut self) -> Result<()> {
        self.varint(10).map(drop)
    }
    fn opt_u32(&mut self) -> Result<()> {
        self.option(|w| w.u32())
    }
    /// BTreeMap<String, Vec<u8>>
    fn proprietary(&mut self) -> Result<()> {
        self.list(|w| {
            w.bytes()?;
            w.bytes()
        })
    }
    fn zip32(&mut self) -> Result<()> {
        self.raw(32)?;
        self.list(|w| w.u32())
    }
    fn witness(&mut self) -> Result<()> {
        self.option(|w| {
            w.u32()?;
            w.raw(32 * 32)
        })
    }

    // ---- layout (pczt 0.10) ----

    fn global(&mut self) -> Result<()> {
        self.u32()?; // tx version
        self.u32()?; // version group
        self.u32()?; // branch
        self.opt_u32()?; // fallback lock time
        self.u32()?; // expiry height
        self.u32()?; // coin type
        self.raw(1)?; // tx modifiable
        self.proprietary()
    }

    fn transparent(&mut self) -> Result<()> {
        self.list(|w| {
            // inputs
            w.raw(32)?;
            w.u32()?;
            w.opt_u32()?;
            w.opt_u32()?;
            w.opt_u32()?;
            w.option(|w| w.bytes())?; // script sig
            w.u64()?; // value
            w.bytes()?; // script pubkey
            w.option(|w| w.bytes())?; // redeem script
            w.list(|w| {
                w.raw(33)?;
                w.bytes()
            })?; // partial signatures
            w.raw(1)?; // sighash type
            w.list(|w| {
                w.raw(33)?;
                w.zip32()
            })?; // bip32 derivation
            w.list(|w| {
                w.raw(20)?;
                w.bytes()
            })?; // ripemd160 preimages
            w.list(|w| {
                w.raw(32)?;
                w.bytes()
            })?; // sha256
            w.list(|w| {
                w.raw(20)?;
                w.bytes()
            })?; // hash160
            w.list(|w| {
                w.raw(32)?;
                w.bytes()
            })?; // hash256
            w.proprietary()
        })?;
        self.list(|w| {
            // outputs
            w.u64()?;
            w.bytes()?;
            w.option(|w| w.bytes())?;
            w.list(|w| {
                w.raw(33)?;
                w.zip32()
            })?;
            w.option(|w| w.bytes())?; // user address
            w.proprietary()
        })
    }

    fn sapling(&mut self, v2: bool) -> Result<()> {
        self.list(|w| {
            // spends
            w.raw(96)?; // cv, nullifier, rk
            w.option(|w| w.raw(192))?; // proof
            w.option(|w| w.raw(64))?; // spend auth sig
            w.option(|w| w.raw(43))?; // recipient
            w.option(|w| w.u64())?; // value
            w.opt32()?; // rcm
            w.opt32()?; // rseed
            w.opt32()?; // rcv
            w.option(|w| w.raw(64))?; // proof generation key
            w.witness()?;
            w.opt32()?; // alpha
            w.option(|w| w.zip32())?;
            w.opt32()?; // dummy ask
            w.proprietary()
        })?;
        self.list(|w| {
            // outputs
            w.raw(96)?; // cv, cmu, ephemeral key
            w.bytes()?; // enc ciphertext
            w.bytes()?; // out ciphertext
            w.option(|w| w.raw(192))?;
            w.option(|w| w.raw(43))?;
            w.option(|w| w.u64())?;
            w.opt32()?; // rseed
            w.opt32()?; // rcv
            w.opt32()?; // ock
            w.option(|w| w.zip32())?;
            w.option(|w| w.bytes())?; // user address
            w.proprietary()
        })?;
        self.varint(19)?; // value sum (i128)
        if v2 {
            self.opt32()?;
        } else {
            self.raw(32)?;
        } // anchor
        self.opt32() // bsk
    }

    fn orchard(&mut self, v2: bool) -> Result<()> {
        self.list(|w| {
            // actions
            if v2 { w.opt32()? } else { w.raw(32)? } // cv net
            // spend
            if v2 {
                w.opt32()?; // nullifier
                w.opt32()?; // rk
            } else {
                w.raw(64)?;
            }
            w.option(|w| w.raw(64))?; // spend auth sig
            w.option(|w| w.raw(43))?; // recipient
            w.option(|w| w.u64())?; // value
            w.opt32()?; // rho
            w.opt32()?; // rseed
            w.added("a split note")?; // rseed_split_note
            w.option(|w| w.raw(96))?; // fvk
            w.witness()?;
            w.opt32()?; // alpha
            w.option(|w| w.zip32())?;
            w.opt32()?; // dummy sk
            w.proprietary()?;
            w.added("an asset")?; // asset
            // output
            if v2 { w.opt32()? } else { w.raw(32)? } // cmx
            w.raw(32)?; // ephemeral key
            if v2 {
                // EncCiphertext: Encrypted | MemoPlaintext, both bytes
                if w.varint(5)? > 1 {
                    return Err(fail("is not a PCZT"));
                }
            }
            w.bytes()?; // enc ciphertext
            w.bytes()?; // out ciphertext
            w.option(|w| w.raw(43))?;
            w.option(|w| w.u64())?;
            w.opt32()?; // rseed
            w.opt32()?; // ock
            w.option(|w| w.zip32())?;
            w.option(|w| w.bytes())?; // user address
            w.proprietary()?;
            w.added("an asset")?; // asset
            w.opt32() // rcv
        })?;
        self.raw(1)?; // flags
        self.u64()?; // value sum (u64)
        self.raw(1)?; // value sum sign
        if v2 {
            self.opt32()?; // anchor
            if self.varint(5)? > 1 {
                return Err(Error::Other("Cupcake signed something with a ZSA note version, which Lock it does not send. Nothing was sent.".into()));
            } // V2 | V3
        } else {
            self.raw(32)?;
        }
        self.option(|w| w.bytes())?; // proof
        self.opt32() // bsk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by the Mac with both crates: pczt 0.10.0 parses each first
    /// string; Cupcake's crate parses each second one.
    const VECTORS: [(&str, &str); 3] = include!("cupcake_vectors.in");

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn the_cupcake_layout_both_ways() {
        for (main, cupcake) in VECTORS {
            assert_eq!(to_cupcake(&bytes(main)).unwrap(), bytes(cupcake));
            assert_eq!(from_cupcake(&bytes(cupcake)).unwrap(), bytes(main));
            let mut short = bytes(main);
            short.pop();
            assert!(to_cupcake(&short).is_err());
        }
        assert!(to_cupcake(b"PSBT1234").is_err());
    }

    #[test]
    fn a_filled_in_asset_is_refused() {
        // Flip the last Orchard "asset" None of a Cupcake vector to Some:
        // the v1 vector ends ...rcv(opt32)... so find an asset byte by
        // re-walking: simplest is to set the issue (last byte of v2) to 1.
        let mut v2 = bytes(VECTORS[0].1);
        *v2.last_mut().unwrap() = 1;
        let err = from_cupcake(&v2).unwrap_err().to_string();
        assert!(err.contains("asset issuance"), "{err}");
    }
}

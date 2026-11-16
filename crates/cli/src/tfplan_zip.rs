//! Reading the prior state's identity out of a saved plan.
//!
//! `terraform plan -out=FILE` (and tofu's) writes a zip. Its `tfstate` entry is the
//! state the plan was computed from, as ordinary state JSON, and the two top-level
//! fields that identify that state are `lineage` and `serial`. `plan push` reads
//! them so it can say - before uploading anything - whether the plan can still be
//! applied to the workspace's current state.
//!
//! Why a hand-rolled reader rather than a zip crate: the CLI needs ONE entry out
//! of a format it otherwise never touches, the server reads the same entry the
//! same way (`lib/tfplanZip.ts`), and keeping the two the same shape matters more
//! than generality. It reads the central directory, not the local headers: Go's
//! zip writer streams entries with a data descriptor, so each local header says 0
//! for both sizes and only the central directory knows the real ones.
//!
//! Nothing here keeps a value from the state. The entry is parsed, three fields
//! are copied out, and the rest is dropped.

use std::io::Read;

/// The zip entry holding the state a plan was computed from.
const PRIOR_STATE_ENTRY: &str = "tfstate";

/// Ceiling on the inflated prior state. A state file is the largest thing in a
/// plan; this only has to be far above any real one and far below "a zip bomb
/// took the runner down".
const MAX_PRIOR_STATE_BYTES: u64 = 512 * 1024 * 1024;

const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_LOCAL: u32 = 0x0403_4b50;
const EOCD_MIN: usize = 22;
const CENTRAL_MIN: usize = 46;
const LOCAL_MIN: usize = 30;

/// The identity of the state a plan was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanPrior {
    /// The state's serial. `None` only if the entry has no integer `serial`.
    pub serial: Option<i64>,
    /// The state's lineage. `None` when the plan was made against NO state: both
    /// tools write an empty string there, and an empty lineage is the one case
    /// neither stale-checks at apply time.
    pub lineage: Option<String>,
    /// The version of the binary that wrote the state.
    pub terraform_version: Option<String>,
}

/// Why a file could not be read as a saved plan.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum TfplanZipError {
    #[error("it is not a zip archive, which every file written by `plan -out=` is")]
    NotAZip,
    #[error("it is a ZIP64 archive, which the plan gate does not read")]
    Zip64,
    #[error("it is a zip with no `tfstate` entry, which every terraform/tofu saved plan has")]
    NoPriorState,
    #[error("its `{0}` entry is truncated or malformed")]
    Malformed(&'static str),
    #[error("its prior state uses zip compression method {0}, which is not supported")]
    UnsupportedMethod(u16),
    #[error("its prior state is larger than the {0}MB this reader will inflate")]
    TooLarge(u64),
    #[error("its prior state failed its checksum; the file is corrupt")]
    BadChecksum,
    #[error("its prior state is not a JSON object")]
    BadJson,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// One central-directory record: where an entry is and how to inflate it.
struct Entry {
    method: u16,
    crc32: u32,
    compressed: usize,
    uncompressed: u64,
    local_header: usize,
}

/// Find `name` in the archive's central directory.
fn find_entry(b: &[u8], name: &str) -> Result<Option<Entry>, TfplanZipError> {
    // The end-of-central-directory record is the last thing in the file, followed
    // only by a comment of at most 65535 bytes. Search backwards for its signature.
    if b.len() < EOCD_MIN {
        return Err(TfplanZipError::NotAZip);
    }
    let floor = b.len().saturating_sub(EOCD_MIN + u16::MAX as usize);
    let eocd = (floor..=b.len() - EOCD_MIN)
        .rev()
        .find(|&i| u32_at(b, i) == Some(SIG_EOCD))
        .ok_or(TfplanZipError::NotAZip)?;

    let total = u16_at(b, eocd + 10).ok_or(TfplanZipError::NotAZip)?;
    let cd_offset = u32_at(b, eocd + 16).ok_or(TfplanZipError::NotAZip)?;
    // ZIP64 parks the real numbers in a second record and leaves these saturated.
    if total == u16::MAX || cd_offset == u32::MAX {
        return Err(TfplanZipError::Zip64);
    }

    let malformed = TfplanZipError::Malformed("central directory");
    let mut at = cd_offset as usize;
    for _ in 0..total {
        if u32_at(b, at) != Some(SIG_CENTRAL) || at + CENTRAL_MIN > b.len() {
            return Err(malformed);
        }
        let name_len = u16_at(b, at + 28).ok_or(malformed.clone())? as usize;
        let extra_len = u16_at(b, at + 30).ok_or(malformed.clone())? as usize;
        let comment_len = u16_at(b, at + 32).ok_or(malformed.clone())? as usize;
        let entry_name = b
            .get(at + CENTRAL_MIN..at + CENTRAL_MIN + name_len)
            .ok_or(malformed.clone())?;
        if entry_name == name.as_bytes() {
            let compressed = u32_at(b, at + 20).ok_or(malformed.clone())?;
            let uncompressed = u32_at(b, at + 24).ok_or(malformed.clone())?;
            let local_header = u32_at(b, at + 42).ok_or(malformed.clone())?;
            if compressed == u32::MAX || uncompressed == u32::MAX || local_header == u32::MAX {
                return Err(TfplanZipError::Zip64);
            }
            return Ok(Some(Entry {
                method: u16_at(b, at + 10).ok_or(malformed.clone())?,
                crc32: u32_at(b, at + 16).ok_or(malformed)?,
                compressed: compressed as usize,
                uncompressed: u64::from(uncompressed),
                local_header: local_header as usize,
            }));
        }
        at += CENTRAL_MIN + name_len + extra_len + comment_len;
    }
    Ok(None)
}

/// Inflate one entry, bounded, and check it against the directory's size and CRC.
fn read_entry(b: &[u8], e: &Entry, label: &'static str) -> Result<Vec<u8>, TfplanZipError> {
    let malformed = TfplanZipError::Malformed(label);
    if e.uncompressed > MAX_PRIOR_STATE_BYTES {
        return Err(TfplanZipError::TooLarge(
            MAX_PRIOR_STATE_BYTES / (1024 * 1024),
        ));
    }
    if u32_at(b, e.local_header) != Some(SIG_LOCAL) {
        return Err(malformed);
    }
    // The local header repeats the name and carries its OWN extra field, whose
    // length need not match the central directory's.
    let name_len = u16_at(b, e.local_header + 26).ok_or(malformed.clone())? as usize;
    let extra_len = u16_at(b, e.local_header + 28).ok_or(malformed.clone())? as usize;
    let start = e.local_header + LOCAL_MIN + name_len + extra_len;
    let data = b
        .get(start..start + e.compressed)
        .ok_or(malformed.clone())?;

    let out = match e.method {
        0 => data.to_vec(),
        8 => {
            let mut out = Vec::with_capacity(e.uncompressed as usize);
            // One byte past the declared size: enough to notice an entry that
            // inflates to more than its directory record admits to.
            flate2::read::DeflateDecoder::new(data)
                .take(e.uncompressed + 1)
                .read_to_end(&mut out)
                .map_err(|_| malformed.clone())?;
            out
        }
        other => return Err(TfplanZipError::UnsupportedMethod(other)),
    };
    if out.len() as u64 != e.uncompressed {
        return Err(malformed);
    }
    let mut crc = flate2::Crc::new();
    crc.update(&out);
    if crc.sum() != e.crc32 {
        return Err(TfplanZipError::BadChecksum);
    }
    Ok(out)
}

/// The lineage and serial of the state a saved plan was computed from.
///
/// The field rules are the server's, deliberately: a `serial` that is not an
/// integer reads as absent, and an empty `lineage` reads as absent.
pub fn read_prior_state(bytes: &[u8]) -> Result<PlanPrior, TfplanZipError> {
    let entry = find_entry(bytes, PRIOR_STATE_ENTRY)?.ok_or(TfplanZipError::NoPriorState)?;
    let raw = read_entry(bytes, &entry, PRIOR_STATE_ENTRY)?;
    let state: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| TfplanZipError::BadJson)?;
    let state = state.as_object().ok_or(TfplanZipError::BadJson)?;
    Ok(PlanPrior {
        serial: state.get("serial").and_then(serde_json::Value::as_i64),
        lineage: state
            .get("lineage")
            .and_then(serde_json::Value::as_str)
            .filter(|l| !l.is_empty())
            .map(str::to_string),
        terraform_version: state
            .get("terraform_version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real saved plans, written by the real binaries. How they were made, and how
    // to remake them: tests/fixtures/tfplan/regenerate.sh.
    const TOFU_FRESH: &[u8] = include_bytes!("../tests/fixtures/tfplan/tofu-fresh.tfplan");
    const TOFU_APPLIED: &[u8] = include_bytes!("../tests/fixtures/tfplan/tofu-applied.tfplan");
    const TERRAFORM_FRESH: &[u8] =
        include_bytes!("../tests/fixtures/tfplan/terraform-fresh.tfplan");
    const TERRAFORM_APPLIED: &[u8] =
        include_bytes!("../tests/fixtures/tfplan/terraform-applied.tfplan");
    const EXPECTED: &str = include_str!("../tests/fixtures/tfplan/expected.json");

    fn expected(name: &str) -> serde_json::Value {
        let all: serde_json::Value = serde_json::from_str(EXPECTED).unwrap();
        all[name].clone()
    }

    /// A plan made after an apply carries that state's lineage and serial - the
    /// same values the local state file had when the fixture was generated.
    #[test]
    fn a_plan_made_against_state_carries_its_lineage_and_serial() {
        for (name, bytes) in [
            ("tofu-applied.tfplan", TOFU_APPLIED),
            ("terraform-applied.tfplan", TERRAFORM_APPLIED),
        ] {
            let want = expected(name);
            let got = read_prior_state(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(got.lineage.as_deref(), want["lineage"].as_str(), "{name}");
            assert_eq!(got.serial, want["serial"].as_i64(), "{name}");
            assert!(got.lineage.is_some(), "{name} should have a lineage");
        }
    }

    /// A plan made against no state has NO lineage. Both tools write `""`, which
    /// must read as absent: that is the case nothing checks at apply time.
    #[test]
    fn a_plan_made_against_no_state_has_no_lineage() {
        for (name, bytes) in [
            ("tofu-fresh.tfplan", TOFU_FRESH),
            ("terraform-fresh.tfplan", TERRAFORM_FRESH),
        ] {
            let got = read_prior_state(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(got.lineage, None, "{name}");
            // The first serial has differed between releases, so only its presence
            // is pinned here.
            assert!(got.serial.is_some(), "{name} should still carry a serial");
        }
    }

    #[test]
    fn the_writing_binary_version_is_read_too() {
        let all: serde_json::Value = serde_json::from_str(EXPECTED).unwrap();
        assert_eq!(
            read_prior_state(TOFU_APPLIED)
                .unwrap()
                .terraform_version
                .as_deref(),
            all["made_with"]["tofu"].as_str()
        );
        assert_eq!(
            read_prior_state(TERRAFORM_APPLIED)
                .unwrap()
                .terraform_version
                .as_deref(),
            all["made_with"]["terraform"].as_str()
        );
    }

    #[test]
    fn things_that_are_not_saved_plans_are_named_as_such() {
        assert_eq!(read_prior_state(b""), Err(TfplanZipError::NotAZip));
        assert_eq!(
            read_prior_state(b"{\"format_version\":\"1.2\"}  this is a show -json render"),
            Err(TfplanZipError::NotAZip)
        );
        // A valid, empty zip: an end-of-central-directory record and nothing else.
        let mut empty = vec![0x50, 0x4b, 0x05, 0x06];
        empty.extend_from_slice(&[0; 18]);
        assert_eq!(read_prior_state(&empty), Err(TfplanZipError::NoPriorState));
    }

    /// A truncated upload must be an error, never a plan with no lineage.
    #[test]
    fn a_truncated_plan_is_refused_not_misread() {
        let cut = &TOFU_APPLIED[..TOFU_APPLIED.len() - 40];
        assert!(read_prior_state(cut).is_err());
    }

    /// Flip one byte inside the compressed prior state: either the inflate fails
    /// or the checksum does, but it never yields a state.
    #[test]
    fn a_corrupted_prior_state_does_not_parse() {
        let entry = find_entry(TOFU_APPLIED, PRIOR_STATE_ENTRY)
            .unwrap()
            .unwrap();
        let name_len = u16_at(TOFU_APPLIED, entry.local_header + 26).unwrap() as usize;
        let extra_len = u16_at(TOFU_APPLIED, entry.local_header + 28).unwrap() as usize;
        let start = entry.local_header + LOCAL_MIN + name_len + extra_len;
        let mut bad = TOFU_APPLIED.to_vec();
        bad[start + entry.compressed / 2] ^= 0xff;
        assert!(read_prior_state(&bad).is_err());
    }

    /// A stored (uncompressed) entry and non-integer / empty fields, built by
    /// hand so the field rules are pinned independently of what the tools emit.
    #[test]
    fn field_rules_match_the_servers() {
        fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let mut z = Vec::new();
            // local header
            z.extend_from_slice(&SIG_LOCAL.to_le_bytes());
            z.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method 0, time, date
            z.extend_from_slice(&crc.sum().to_le_bytes());
            z.extend_from_slice(&(data.len() as u32).to_le_bytes());
            z.extend_from_slice(&(data.len() as u32).to_le_bytes());
            z.extend_from_slice(&(name.len() as u16).to_le_bytes());
            z.extend_from_slice(&0u16.to_le_bytes());
            z.extend_from_slice(name.as_bytes());
            z.extend_from_slice(data);
            let cd_offset = z.len() as u32;
            // central directory record
            z.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
            z.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // made by, needed, flags, method, time, date
            z.extend_from_slice(&crc.sum().to_le_bytes());
            z.extend_from_slice(&(data.len() as u32).to_le_bytes());
            z.extend_from_slice(&(data.len() as u32).to_le_bytes());
            z.extend_from_slice(&(name.len() as u16).to_le_bytes());
            z.extend_from_slice(&[0; 12]); // extra, comment, disk, internal, external
            z.extend_from_slice(&0u32.to_le_bytes()); // local header offset
            z.extend_from_slice(name.as_bytes());
            let cd_size = z.len() as u32 - cd_offset;
            // end of central directory
            z.extend_from_slice(&SIG_EOCD.to_le_bytes());
            z.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
            z.extend_from_slice(&cd_size.to_le_bytes());
            z.extend_from_slice(&cd_offset.to_le_bytes());
            z.extend_from_slice(&0u16.to_le_bytes());
            z
        }

        let ok = stored_zip("tfstate", br#"{"serial":7,"lineage":"abc"}"#);
        assert_eq!(
            read_prior_state(&ok).unwrap(),
            PlanPrior {
                serial: Some(7),
                lineage: Some("abc".into()),
                terraform_version: None
            }
        );
        let odd = stored_zip("tfstate", br#"{"serial":"7","lineage":""}"#);
        let got = read_prior_state(&odd).unwrap();
        assert_eq!((got.serial, got.lineage), (None, None));

        assert_eq!(
            read_prior_state(&stored_zip("tfstate", b"[1,2]")),
            Err(TfplanZipError::BadJson)
        );
        assert_eq!(
            read_prior_state(&stored_zip("not-tfstate", b"{}")),
            Err(TfplanZipError::NoPriorState)
        );
    }
}

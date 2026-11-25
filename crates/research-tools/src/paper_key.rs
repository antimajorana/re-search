//! Shared paper identity (package F2): `PaperKey`, `PaperRef`, `NodeRef`, the reserved
//! id-prefix table of the three modes, and Investigate's permanent node tag.
//!
//! Nothing here touches the network, the ledger or any tool definition. Everything is pure and
//! deterministic, because node tags derived from a `PaperKey` are permanent.
use crate::tex_scan::{normalize_arxiv_id, normalize_doi};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_VERSION_CHARS: usize = 32;
const MAX_INSPIRE_DIGITS: usize = 12;
const MAX_OPENALEX_DIGITS: usize = 15;
const S2_ID_HEX: usize = 40;
const FILE_KEY_HEX: usize = 16;
const TAG_CHARS: usize = 10;
/// Field separator of the node-tag hash input: ASCII unit separator, one byte.
pub const TAG_FIELD_SEPARATOR: char = '\u{1f}';

/// Which identifier a `PaperKey` is built from. The declaration order is the order of
/// preference ("first available identifier"); `File` is last and only for dropped files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PaperKeyKind { Arxiv, Doi, Inspire, S2, Openalex, File }

impl PaperKeyKind {
    pub const ALL: [PaperKeyKind; 6] =
        [Self::Arxiv, Self::Doi, Self::Inspire, Self::S2, Self::Openalex, Self::File];

    /// The text before the colon in the key.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Arxiv => "arxiv",
            Self::Doi => "doi",
            Self::Inspire => "inspire",
            Self::S2 => "s2",
            Self::Openalex => "openalex",
            Self::File => "file",
        }
    }
}

/// The one name of a paper in every mode. Always in canonical form:
///
/// - `arxiv:<base id, no version>`, lower case, old-style ids without subject class
/// - `doi:<normalised>` (see [`canonical_doi`])
/// - `inspire:<recid>`, decimal without leading zeros
/// - `s2:<paperId>`, 40 lower-case hex characters
/// - `openalex:<W id>`, `W` followed by decimal digits
/// - `file:<first 16 hex of sha256 of the raw file>`
///
/// Serialised as a plain JSON string. Deserialising validates with [`PaperKey::parse`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaperKey { text: String, kind: PaperKeyKind }

impl PaperKey {
    fn build(kind: PaperKeyKind, value: &str) -> Self {
        Self { text: format!("{}:{value}", kind.prefix()), kind }
    }

    /// From any accepted spelling of an arXiv id; a version suffix is dropped.
    /// Use [`split_arxiv_id`] when the version is needed.
    pub fn from_arxiv(raw: &str) -> Result<Self, String> {
        split_arxiv_id(raw).map(|(base, _)| Self::build(PaperKeyKind::Arxiv, &base))
    }

    pub fn from_doi(raw: &str) -> Result<Self, String> {
        canonical_doi(raw).map(|doi| Self::build(PaperKeyKind::Doi, &doi))
    }

    /// INSPIRE-HEP record id: decimal digits (surrounding whitespace is trimmed).
    pub fn from_inspire(raw: &str) -> Result<Self, String> {
        canonical_inspire(raw).map(|id| Self::build(PaperKeyKind::Inspire, &id))
    }

    /// Semantic Scholar `paperId`: 40 hex characters; upper case is folded to lower case.
    pub fn from_s2(raw: &str) -> Result<Self, String> {
        canonical_s2(raw).map(|id| Self::build(PaperKeyKind::S2, &id))
    }

    /// OpenAlex work id: `W<digits>`, optionally as an `https://openalex.org/` link.
    pub fn from_openalex(raw: &str) -> Result<Self, String> {
        canonical_openalex(raw).map(|id| Self::build(PaperKeyKind::Openalex, &id))
    }

    /// For a dropped or attached file: the first 16 hex characters of the SHA-256 of the
    /// raw bytes, exactly as received (before any decompression or text extraction).
    pub fn from_file_bytes(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let hex: String = digest[..FILE_KEY_HEX / 2].iter().map(|b| format!("{b:02x}")).collect();
        Self::build(PaperKeyKind::File, &hex)
    }

    /// Strict parser: accepts only a key that is already canonical, byte for byte.
    /// `arxiv:2103.01234v2`, `doi:10.1000/ABC` or `arXiv:2103.01234` are rejected; use the
    /// `from_*` constructors to normalise loose input.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (prefix, value) = text
            .split_once(':')
            .ok_or_else(|| format!("paper key `{}` has no `<kind>:` prefix", shown(text)))?;
        let kind = PaperKeyKind::ALL
            .into_iter()
            .find(|k| k.prefix() == prefix)
            .ok_or_else(|| format!("paper key `{}` has an unknown kind", shown(text)))?;
        let canonical = match kind {
            PaperKeyKind::Arxiv => split_arxiv_id(value).and_then(|(base, version)| match version {
                Some(_) => Err("an arXiv paper key must not carry a version".to_string()),
                None => Ok(base),
            }),
            PaperKeyKind::Doi => canonical_doi(value),
            PaperKeyKind::Inspire => canonical_inspire(value),
            PaperKeyKind::S2 => canonical_s2(value),
            PaperKeyKind::Openalex => canonical_openalex(value),
            PaperKeyKind::File => {
                if value.len() == FILE_KEY_HEX && is_lower_hex(value) {
                    Ok(value.to_string())
                } else {
                    Err(format!("a file paper key needs exactly {FILE_KEY_HEX} lower-case hex characters"))
                }
            }
        }?;
        if canonical != value {
            return Err(format!(
                "paper key `{}` is not canonical (expected `{prefix}:{canonical}`)",
                shown(text)
            ));
        }
        Ok(Self { text: text.to_string(), kind })
    }

    pub fn kind(&self) -> PaperKeyKind { self.kind }
    /// The part after `<kind>:`.
    pub fn value(&self) -> &str { &self.text[self.kind.prefix().len() + 1..] }
    pub fn as_str(&self) -> &str { &self.text }
}

impl fmt::Display for PaperKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.text) }
}

impl std::str::FromStr for PaperKey {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> { Self::parse(s) }
}

impl Serialize for PaperKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for PaperKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

fn shown(text: &str) -> String { text.chars().take(80).collect() }
fn is_lower_hex(s: &str) -> bool { s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) }

/// Splits any accepted spelling of an arXiv id into `(canonical base id, version)`, for
/// example `"arXiv:2103.01234v2"` to `("2103.01234", Some("v2"))`.
///
/// Accepted spellings are exactly those of the existing [`normalize_arxiv_id`] (bare id,
/// `arXiv:` prefix, abs / pdf / e-print links, optional `vN`), which is reused unchanged.
/// On top of it the canonical base id is:
///
/// - lower case;
/// - new-style `YYMM.NNNN` (0704 to 1412) or `YYMM.NNNNN` (from 1501), with a real month.
///   A digit count that does not match the month is rejected, not repaired;
/// - old-style `archive/YYMMNNN` (9107 to 0703) **without the subject class**:
///   `math.AG/0601001` and `math/0601001` are the same paper, since the number is unique
///   within the archive and the subject class is only a label, so both give
///   `math/0601001`. Archive names are checked for shape only (letters and hyphens), not
///   against a list of arXiv archives.
pub fn split_arxiv_id(raw: &str) -> Result<(String, Option<String>), String> {
    let bad = |why: &str| format!("`{}` is not an arXiv id: {why}", shown(raw));
    let norm = normalize_arxiv_id(raw).ok_or_else(|| bad("unrecognised form"))?.to_ascii_lowercase();
    let (base, version) = match norm.rfind('v') {
        Some(i) if i + 1 < norm.len() && norm[i + 1..].bytes().all(|b| b.is_ascii_digit()) => {
            (&norm[..i], Some(norm[i..].to_string()))
        }
        _ => (norm.as_str(), None),
    };
    let yymm = |digits: &str| -> Result<u32, String> {
        let value: u32 = digits[..4].parse().map_err(|_| bad("unreadable date"))?;
        if !(1..=12).contains(&(value % 100)) {
            return Err(bad("month is not 01 to 12"));
        }
        Ok(value)
    };
    let canonical = match base.split_once('/') {
        Some((archive, number)) => {
            let date = yymm(number)?;
            // Old-style ids were issued from 1991 until March 2007.
            if !(date >= 9107 || date <= 703) {
                return Err(bad("old-style ids end at 0703"));
            }
            let archive = archive.split('.').next().unwrap_or(archive);
            if archive.is_empty() || archive.starts_with('-') || archive.ends_with('-') {
                return Err(bad("bad archive name"));
            }
            format!("{archive}/{number}")
        }
        None => {
            let (date_part, number) = base.split_once('.').ok_or_else(|| bad("unrecognised form"))?;
            let date = yymm(date_part)?;
            if date < 704 {
                return Err(bad("new-style ids start at 0704"));
            }
            let want = if date <= 1412 { 4 } else { 5 };
            if number.len() != want {
                return Err(bad("wrong number of digits for that month"));
            }
            base.to_string()
        }
    };
    Ok((canonical, version))
}

/// Normalised DOI, consistent with what `literature_search` already returns: this is the
/// existing `tex_scan::normalize_doi` (trim, percent-decode, strip `https://doi.org/`,
/// `http://dx.doi.org/` and `doi:`, drop trailing punctuation, require `10.<digits>/<suffix>`,
/// ASCII lower case), plus one stricter rule: no whitespace or control character inside.
pub fn canonical_doi(raw: &str) -> Result<String, String> {
    let doi = normalize_doi(raw).ok_or_else(|| format!("`{}` is not a DOI", shown(raw)))?;
    if doi.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("`{}` is not a DOI: whitespace inside", shown(raw)));
    }
    Ok(doi)
}

fn canonical_decimal(raw: &str, max_digits: usize) -> Option<String> {
    let s = raw.trim();
    let ok = !s.is_empty() && s.len() <= max_digits && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0');
    ok.then(|| s.to_string())
}

fn canonical_inspire(raw: &str) -> Result<String, String> {
    canonical_decimal(raw, MAX_INSPIRE_DIGITS)
        .ok_or_else(|| format!("`{}` is not an INSPIRE record id (digits, no leading zero)", shown(raw)))
}

fn canonical_s2(raw: &str) -> Result<String, String> {
    let s = raw.trim().to_ascii_lowercase();
    if s.len() == S2_ID_HEX && is_lower_hex(&s) {
        Ok(s)
    } else {
        Err(format!("`{}` is not a Semantic Scholar paperId ({S2_ID_HEX} hex characters)", shown(raw)))
    }
}

fn canonical_openalex(raw: &str) -> Result<String, String> {
    let mut s = raw.trim();
    for prefix in ["https://openalex.org/works/", "https://openalex.org/"] {
        if s.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix)) {
            s = &s[prefix.len()..];
            break;
        }
    }
    s.strip_prefix(['W', 'w'])
        .and_then(|digits| canonical_decimal(digits, MAX_OPENALEX_DIGITS).filter(|d| d == digits))
        .map(|digits| format!("W{digits}"))
        .ok_or_else(|| format!("`{}` is not an OpenAlex work id (W and digits)", shown(raw)))
}

/// The other identifiers of a paper. Field set as in the overview, section 2.3:
/// `{arxiv, doi, inspire, s2}`. Values are bare (no `arxiv:` prefix) and canonical when
/// produced by [`PaperRef::from_ids`]. OpenAlex is deliberately not a field here: the spec
/// lists it as a `PaperKey` form but not among `ids`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperIds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arxiv: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspire: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s2: Option<String>,
}

/// A paper with an optional version and its known identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperRef {
    pub key: PaperKey,
    /// For arXiv papers `"v3"`. Omitted from JSON when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub ids: PaperIds,
}

impl PaperRef {
    /// Key = first available identifier in the order arxiv, doi, inspire, s2. Errors when
    /// none is given or when a given identifier is malformed (it is never skipped silently).
    pub fn from_ids(ids: PaperIds, version: Option<String>) -> Result<Self, String> {
        Self::from_ids_or(ids, version, None)
    }

    /// As [`PaperRef::from_ids`], with a key to fall back on when `ids` is empty: an
    /// `openalex:` key (fifth in the order) or a `file:` key (dropped file). The fallback
    /// is used only when none of the four identifiers is present.
    ///
    /// The identifiers are stored canonical. If the arXiv id carries a version
    /// (`2103.01234v2`) and `version` is `None`, that version is kept; an explicit
    /// `version` wins. For an arXiv key the version must look like `v3`.
    pub fn from_ids_or(ids: PaperIds, version: Option<String>, fallback: Option<PaperKey>) -> Result<Self, String> {
        let present = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let (arxiv, arxiv_version) = match present(&ids.arxiv) {
            Some(raw) => split_arxiv_id(&raw).map(|(base, v)| (Some(base), v))?,
            None => (None, None),
        };
        let ids = PaperIds {
            arxiv,
            doi: present(&ids.doi).map(|raw| canonical_doi(&raw)).transpose()?,
            inspire: present(&ids.inspire).map(|raw| canonical_inspire(&raw)).transpose()?,
            s2: present(&ids.s2).map(|raw| canonical_s2(&raw)).transpose()?,
        };
        let key = [
            (PaperKeyKind::Arxiv, &ids.arxiv),
            (PaperKeyKind::Doi, &ids.doi),
            (PaperKeyKind::Inspire, &ids.inspire),
            (PaperKeyKind::S2, &ids.s2),
        ]
        .into_iter()
        .find_map(|(kind, value)| value.as_deref().map(|v| PaperKey::build(kind, v)))
        .or(fallback)
        .ok_or_else(|| "a paper reference needs at least one identifier".to_string())?;
        let version = match version.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
            Some(v) => Some(v),
            None => arxiv_version,
        };
        if let Some(v) = &version {
            let arxiv_form = v.strip_prefix('v').is_some_and(|n| {
                !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())
            });
            let plain = v.chars().count() <= MAX_VERSION_CHARS && !v.chars().any(|c| c.is_whitespace() || c.is_control());
            if !plain || (key.kind() == PaperKeyKind::Arxiv && !arxiv_form) {
                return Err(format!("`{}` is not a paper version (expected a form like `v3`)", shown(v)));
            }
        }
        Ok(Self { key, version, ids })
    }
}

/// The mode a session or node belongs to.
///
/// Wire names are the lower-case on-screen names: `"investigate" | "explore" | "solve"` are
/// exactly the values of `handoff.packet.target_mode` (overview 4.1), and `NodeRef.mode`
/// sits next to it in the same packet. `"research"` is the tag of an earlier session with
/// no mode marker (overview 5.2); it can be the origin of a pointer but never a hand-over
/// target. The internal event prefixes (`investigation.*`, `idea.*`, `solve.*`) are not
/// used as mode names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode { Investigate, Explore, Solve, Research }

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Investigate => "investigate",
            Self::Explore => "explore",
            Self::Solve => "solve",
            Self::Research => "research",
        }
    }
}

/// A pointer to a node of some session. A pointer that crosses sessions is always this,
/// never a bare id. `node_fp` is the fingerprint of the node body at the time of pointing,
/// so the reader can tell that the node changed since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRef {
    pub session: String,
    pub mode: Mode,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<PaperRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_fp: Option<String>,
}

/// One reserved id prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdPrefix {
    pub prefix: &'static str,
    pub mode: Mode,
    pub meaning: &'static str,
    pub scope: &'static str,
}

const PER_SESSION: &str = "per session";
const fn reserved(prefix: &'static str, mode: Mode, meaning: &'static str, scope: &'static str) -> IdPrefix {
    IdPrefix { prefix, mode, meaning, scope }
}

/// The reserved id prefixes of all modes (overview 2.3). An id is matched on its start, so
/// `e-` (Solve) and `xe-` (Explore) do not interfere. Tests check that no prefix occurs
/// twice and that none is the start of another.
///
/// Not in the table, on purpose:
/// - Solve's `name@version` handles and its check ids carry no prefix. A handle whose name
///   happens to begin with a reserved prefix (say `c-1@2`) would be attributed to the wrong
///   mode by [`mode_of_id`]; that is why a pointer across sessions is a [`NodeRef`], which
///   states the mode, and why `mode_of_id` is only a hint for bare ids.
/// - Earlier Research sessions have no reserved prefix.
///
/// `xe-` and `xa-` are listed under Explore as in 2.3, but by 2.5 they are the ids of the
/// shared events `evidence.quote` and `evidence.abstract`, so they can occur in any mode.
pub const ID_PREFIXES: &[IdPrefix] = &[
    reserved("n-", Mode::Investigate, "skeleton node tag", "stable across sessions and versions of one paper"),
    reserved("m-", Mode::Investigate, "model-created node", PER_SESSION),
    reserved("legacy:", Mode::Investigate, "node of an earlier map, as legacy:<seq>:<id>", PER_SESSION),
    reserved("sc-", Mode::Explore, "part of the idea (subclaim)", PER_SESSION),
    reserved("p-", Mode::Explore, "perspective", PER_SESSION),
    reserved("c-", Mode::Explore, "map node: concept", PER_SESSION),
    reserved("d-", Mode::Explore, "map node: definition", PER_SESSION),
    reserved("k-", Mode::Explore, "map node: known result", PER_SESSION),
    reserved("q-", Mode::Explore, "map node: open question", PER_SESSION),
    reserved("w-", Mode::Explore, "map node: related work", PER_SESSION),
    reserved("r-", Mode::Explore, "map node: direction", PER_SESSION),
    reserved("xe-", Mode::Explore, "stored quote (evidence.quote), xe-<seq>", "per session; shared event id"),
    reserved("xa-", Mode::Explore, "stored abstract quote (evidence.abstract), xa-<seq>", "per session; shared event id"),
    reserved("e-", Mode::Solve, "step", PER_SESSION),
];

/// The table entry whose prefix starts `id`, if any.
pub fn id_prefix_of(id: &str) -> Option<&'static IdPrefix> {
    ID_PREFIXES.iter().find(|entry| id.len() > entry.prefix.len() && id.starts_with(entry.prefix))
}

/// The mode whose reserved prefix starts `id`. `None` for ids without a reserved prefix
/// (Solve handles and check ids, Research ids) and for a bare prefix with nothing after it.
pub fn mode_of_id(id: &str) -> Option<Mode> { id_prefix_of(id).map(|entry| entry.mode) }

/// RFC 4648 base32 with the lower-case alphabet `a-z2-7` and no `=` padding.
pub fn base32_lower_nopad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Investigate's permanent skeleton tag:
/// `"n-" + base32(sha256(PaperKey | kind class | key))[..10]`.
///
/// Fixed here, and pinned by golden tests, because tags are permanent:
/// - hash input: the UTF-8 bytes of `paper_key`, `kind_class` and `key`, joined by one
///   byte 0x1F ([`TAG_FIELD_SEPARATOR`]); no trailing separator, no Unicode normalisation;
/// - base32: RFC 4648 alphabet in lower case (`a-z2-7`), no padding; the first 10
///   characters are the first 50 bits of the digest.
///
/// The join is unambiguous: a `PaperKey` never contains a control character, `kind_class`
/// must be non-empty `[a-z0-9_]`, and `key` is the last field (it must be non-empty; it may
/// contain any character, since nothing follows it). What the kind classes are, and the
/// `label:` / `fp:` form of `key`, belong to the skeleton builder, not to this function.
pub fn node_tag(paper_key: &PaperKey, kind_class: &str, key: &str) -> Result<String, String> {
    if kind_class.is_empty() || !kind_class.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_')) {
        return Err(format!("kind class `{}` must be non-empty lower-case letters, digits or `_`", shown(kind_class)));
    }
    if key.is_empty() {
        return Err("node key must not be empty".to_string());
    }
    let mut hasher = Sha256::new();
    hasher.update(paper_key.as_str().as_bytes());
    hasher.update([TAG_FIELD_SEPARATOR as u8]);
    hasher.update(kind_class.as_bytes());
    hasher.update([TAG_FIELD_SEPARATOR as u8]);
    hasher.update(key.as_bytes());
    let encoded = base32_lower_nopad(&hasher.finalize());
    Ok(format!("n-{}", &encoded[..TAG_CHARS]))
}

#[cfg(test)]
#[path = "paper_key_tests.rs"]
mod tests;

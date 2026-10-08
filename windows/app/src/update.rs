//! In-app updates, platform-neutral part: reading the GitHub releases list,
//! choosing the newest suitable release and its asset, and verifying the
//! download against the published SHA-256. The network and install steps
//! live in `updater.rs` (Windows only). Unit-tested on any OS.

/// GitHub releases API for this project (newest first).
pub const RELEASES_URL: &str =
    "https://api.github.com/repos/kyan0s-git/mem-manager/releases?per_page=20";

/// Automatic checks: once shortly after start, then daily.
pub const FIRST_CHECK_DELAY_MS: u64 = 2 * 60_000;
pub const CHECK_EVERY_MS: u64 = 24 * 3_600_000;

// ------------------------------------------------------------------- JSON

/// A minimal JSON value: enough to read the releases API.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => &[],
        }
    }

    pub fn parse(text: &str) -> Option<Json> {
        let mut p = Parser {
            b: text.as_bytes(),
            i: 0,
            depth: 0,
        };
        let v = p.value()?;
        p.ws();
        (p.i == p.b.len()).then_some(v)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn lit(&mut self, word: &str, v: Json) -> Option<Json> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Some(v)
        } else {
            None
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.b.get(self.i)? {
            b'{' => {
                self.nest()?;
                self.i += 1;
                let mut kv = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.ws();
                        let k = self.string()?;
                        if !self.eat(b':') {
                            return None;
                        }
                        kv.push((k, self.value()?));
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::Obj(kv))
            }
            b'[' => {
                self.nest()?;
                self.i += 1;
                let mut a = Vec::new();
                if !self.eat(b']') {
                    loop {
                        a.push(self.value()?);
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::Arr(a))
            }
            b'"' => self.string().map(Json::Str),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'n' => self.lit("null", Json::Null),
            _ => self.number(),
        }
    }

    fn nest(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= 64).then_some(())
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.i;
        while self.i < self.b.len()
            && matches!(
                self.b[self.i],
                b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
            )
        {
            self.i += 1;
        }
        std::str::from_utf8(&self.b[start..self.i])
            .ok()?
            .parse()
            .ok()
            .map(Json::Num)
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp)
                                && self.b[self.i..].starts_with(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                cp = 0x10000
                                    + ((cp - 0xD800) << 10)
                                    + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            char::from_u32(cp).unwrap_or('\u{FFFD}')
                        }
                        _ => return None,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
    }
}

// --------------------------------------------------------------- versions

/// A semantic version: `0.2.0`, `v1.0.0-rc1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (s, None),
        };
        let mut it = core.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next().unwrap_or("0").parse().ok()?;
        let patch = it.next().unwrap_or("0").parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// 0.x versions and suffixed versions are pre-releases.
    pub fn is_prerelease(&self) -> bool {
        self.major == 0 || self.pre.is_some()
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(o.major, o.minor, o.patch))
            .then_with(|| match (&self.pre, &o.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

// --------------------------------------------------------------- releases

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    pub prerelease: bool,
    pub draft: bool,
    pub html_url: String,
    pub assets: Vec<Asset>,
}

pub fn parse_releases(json: &str) -> Option<Vec<Release>> {
    let v = Json::parse(json)?;
    let Json::Arr(items) = v else { return None };
    Some(
        items
            .iter()
            .filter_map(|r| {
                Some(Release {
                    version: Version::parse(r.get("tag_name")?.str()?)?,
                    prerelease: r.get("prerelease").and_then(Json::bool).unwrap_or(false),
                    draft: r.get("draft").and_then(Json::bool).unwrap_or(false),
                    html_url: r
                        .get("html_url")
                        .and_then(Json::str)
                        .unwrap_or_default()
                        .to_string(),
                    assets: r
                        .get("assets")
                        .map(Json::arr)
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|a| {
                            Some(Asset {
                                name: a.get("name")?.str()?.to_string(),
                                url: a.get("browser_download_url")?.str()?.to_string(),
                                size: a.get("size").and_then(Json::num).unwrap_or(0.0) as u64,
                            })
                        })
                        .collect(),
                })
            })
            .collect(),
    )
}

/// A newer release this installation can update to.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub version: Version,
    pub html_url: String,
    pub asset: Asset,
    /// The `<asset>.sha256` file published next to it.
    pub checksum: Asset,
}

/// The newest non-draft release newer than `current` that carries the asset
/// `asset_name(version)` and its `.sha256`. Pre-releases are offered only to
/// pre-release installs (0.x or suffixed versions).
pub fn choose(
    releases: &[Release],
    current: &Version,
    asset_name: impl Fn(&Version) -> String,
) -> Option<Offer> {
    let allow_pre = current.is_prerelease();
    releases
        .iter()
        .filter(|r| !r.draft && (allow_pre || !r.prerelease) && r.version > *current)
        .filter_map(|r| {
            let name = asset_name(&r.version);
            let asset = r.assets.iter().find(|a| a.name == name)?;
            let sum = r
                .assets
                .iter()
                .find(|a| a.name == format!("{name}.sha256"))?;
            Some(Offer {
                version: r.version.clone(),
                html_url: r.html_url.clone(),
                asset: asset.clone(),
                checksum: sum.clone(),
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// What the UI shows about updates.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate {
        at_ms: u64,
    },
    Available(Offer),
    Installing(Version),
    Failed(String),
}

impl Status {
    pub fn busy(&self) -> bool {
        matches!(self, Status::Checking | Status::Installing(_))
    }
}

// ----------------------------------------------------------------- SHA-256

/// SHA-256 (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut tail = Vec::with_capacity(128);
    let full = data.len() / 64 * 64;
    tail.extend_from_slice(&data[full..]);
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&bit_len.to_be_bytes());
    let mut w = [0u32; 64];
    for block in data[..full].chunks_exact(64).chain(tail.chunks_exact(64)) {
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest in a `sha256sum`-style file: "<64 hex>  <file name>".
pub fn parse_checksum(text: &str) -> Option<String> {
    let token = text.split_whitespace().next()?.to_ascii_lowercase();
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(token)
}

/// Does `data` match the published checksum file?
pub fn verify(data: &[u8], checksum_file: &str) -> bool {
    parse_checksum(checksum_file).is_some_and(|want| hex(&sha256(data)) == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&sha256(&million)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn checksum_files() {
        let f = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  abc.txt\n";
        assert!(verify(b"abc", f));
        assert!(!verify(b"abd", f));
        assert!(!verify(b"abc", "nope"));
        assert!(!verify(b"abc", ""));
    }

    #[test]
    fn versions() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("v0.2.0") > v("0.1.9"));
        assert!(v("1.0.0") > v("1.0.0-rc1"));
        assert!(v("1.0.0-rc2") > v("1.0.0-rc1"));
        assert!(v("0.1") == v("0.1.0"));
        assert!(Version::parse("x.y").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(v("0.3.0").is_prerelease());
        assert!(!v("1.0.0").is_prerelease());
        assert_eq!(v("v1.2.3-beta").to_string(), "1.2.3-beta");
    }

    #[test]
    fn json() {
        let j = Json::parse(r#" {"a": [1, -2.5e1, true, null, "x\"yé😀"], "b": {}} "#).unwrap();
        let a = j.get("a").unwrap().arr();
        assert_eq!(a[1].num(), Some(-25.0));
        assert_eq!(a[4].str(), Some("x\"y\u{e9}\u{1F600}"));
        assert!(Json::parse("[1,]").is_none());
        assert!(Json::parse("{\"a\":1} x").is_none());
        assert!(Json::parse(&"[".repeat(100)).is_none());
    }

    const SAMPLE: &str = r#"[
      {"tag_name":"v0.3.0-rc1","draft":false,"prerelease":true,"html_url":"https://x/0.3.0-rc1",
       "assets":[{"name":"MemManager-0.3.0-rc1-windows-setup.exe","browser_download_url":"https://d/a","size":10},
                 {"name":"MemManager-0.3.0-rc1-windows-setup.exe.sha256","browser_download_url":"https://d/a.sha256","size":1}]},
      {"tag_name":"v0.4.0","draft":true,"prerelease":true,"html_url":"","assets":[]},
      {"tag_name":"v0.2.0","draft":false,"prerelease":true,"html_url":"https://x/0.2.0",
       "assets":[{"name":"MemManager-0.2.0-windows-setup.exe","browser_download_url":"https://d/b","size":20},
                 {"name":"MemManager-0.2.0-windows-setup.exe.sha256","browser_download_url":"https://d/b.sha256","size":1},
                 {"name":"MemManager-0.2.0-windows-x64.exe","browser_download_url":"https://d/c","size":5}]},
      {"tag_name":"v0.1.0","draft":false,"prerelease":true,"html_url":"https://x/0.1.0","assets":[]}
    ]"#;

    #[test]
    fn choose_newest_with_checksummed_asset() {
        let rel = parse_releases(SAMPLE).unwrap();
        assert_eq!(rel.len(), 4);
        let setup = |v: &Version| format!("MemManager-{v}-windows-setup.exe");
        let cur = Version::parse("0.1.0").unwrap();
        let o = choose(&rel, &cur, setup).unwrap();
        assert_eq!(o.version.to_string(), "0.3.0-rc1");
        assert_eq!(o.asset.url, "https://d/a");
        assert_eq!(o.checksum.url, "https://d/a.sha256");
        // The portable exe of 0.2.0 has no checksum file, so it is not offered.
        let portable = |v: &Version| format!("MemManager-{v}-windows-x64.exe");
        assert!(choose(&rel, &cur, portable).is_none());
        // Up to date, and stable installs never get pre-releases.
        assert!(choose(&rel, &Version::parse("0.3.0").unwrap(), setup).is_none());
        assert!(choose(&rel, &Version::parse("1.0.0").unwrap(), setup).is_none());
    }
}

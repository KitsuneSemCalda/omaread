//! AZW3 (KF8) and MOBI (MOBI 6), read in place.
//!
//! Both are the same container: a PalmDB whose text records are compressed with
//! PalmDoc LZ77 or HUFF/CDIC, followed by image records. KF8 stores each XHTML
//! file as a skeleton plus fragments to splice into it; MOBI 6 stores one HTML
//! stream with `<mbp:pagebreak>` between chapters and `filepos=` byte offsets
//! for links. Either way the result is a handful of XHTML parts and images under
//! archive-style hrefs, so everything above `Book` reads them like an EPUB.
//!
//! Layouts follow calibre's `ebooks/mobi/reader` and KindleUnpack.
//!
//! ponytail: a hybrid `.mobi` (MOBI 6 + KF8 after a BOUNDARY record) is read
//! through its MOBI 6 half. Follow EXTH 121 to the KF8 half if one renders badly.

use std::collections::HashMap;
use std::path::Path;

const NULL: u32 = 0xFFFF_FFFF;

/// A decoded book, laid out as if it were an archive.
pub struct Unpacked {
    pub title: String,
    pub author: String,
    pub language: String,
    /// `/part0000.html`, … in reading order.
    pub spine: Vec<String>,
    /// (label, depth, href with an optional `#fragment`), in document order.
    pub toc: Vec<(String, usize, String)>,
    /// Every part and image, by href. Images are `/res/00001`, numbered as the
    /// book numbers them.
    pub files: HashMap<String, Vec<u8>>,
    pub cover: Option<String>,
}

pub fn open(path: &Path) -> Result<Unpacked, String> {
    let data = std::fs::read(path).map_err(|e| format!("read: {e}"))?;
    unpack(&data)
}

fn unpack(data: &[u8]) -> Result<Unpacked, String> {
    if data.get(60..68) != Some(b"BOOKMOBI") {
        return Err("not a MOBI file".into());
    }
    let recs = records(data).ok_or("truncated PalmDB")?;
    let r0 = *recs.first().ok_or("no records")?;
    if r0.get(16..20) != Some(b"MOBI") {
        return Err("no MOBI header".into());
    }
    if u16_at(r0, 12) != Some(0) {
        return Err("DRM-protected".into());
    }
    let compression = u16_at(r0, 0).unwrap_or(0);
    let text_count = u16_at(r0, 8).unwrap_or(0) as usize;
    let header_end = 16 + u32_at(r0, 20).unwrap_or(0) as usize;
    let utf8 = u32_at(r0, 28) == Some(65001);
    let version = u32_at(r0, 36).unwrap_or(0);
    // A field past the end of a short header is absent, not garbage.
    let at = |o: usize| match o + 4 <= header_end {
        true => u32_at(r0, o).unwrap_or(NULL),
        false => NULL,
    };
    let extra_flags = match 0xF4 <= header_end {
        true => u16_at(r0, 0xF2).unwrap_or(0),
        false => 0,
    };

    let mut exth: HashMap<u32, &[u8]> = HashMap::new();
    if u32_at(r0, 0x80).unwrap_or(0) & 0x40 != 0 {
        let e = r0.get(header_end..).unwrap_or_default();
        if e.starts_with(b"EXTH") {
            let mut o = 12;
            for _ in 0..u32_at(e, 8).unwrap_or(0) {
                let (Some(t), Some(l)) = (u32_at(e, o), u32_at(e, o + 4)) else { break };
                let Some(v) = e.get(o + 8..o + l as usize) else { break };
                exth.entry(t).or_insert(v);
                o += (l as usize).max(8);
            }
        }
    }

    let mut huff = match compression {
        1 | 2 => None,
        17480 => {
            let (start, n) = (at(0x70) as usize, at(0x74) as usize);
            let tables = recs.get(start..start.saturating_add(n)).ok_or("missing HUFF records")?;
            Some(Huff::new(tables).ok_or("bad HUFF/CDIC tables")?)
        }
        c => return Err(format!("unknown compression {c}")),
    };
    let mut text = Vec::new();
    for rec in recs.get(1..=text_count).ok_or("missing text records")? {
        let body = &rec[..rec.len() - trailing(rec, extra_flags)];
        match &mut huff {
            Some(h) => h.unpack(body, &mut text, 0).ok_or("corrupt HUFF text")?,
            None if compression == 2 => palmdoc(body, &mut text),
            None => text.extend_from_slice(body),
        }
    }

    // Table of contents: (label, depth, the entry's tags).
    let ncx: Vec<(String, usize, Tags)> = read_index(&recs, at(0xF4) as usize)
        .map(|(entries, cncx)| {
            entries
                .into_iter()
                .filter_map(|e| {
                    let label = decode(cncx_str(&cncx, *e.tags.get(&3)?.first()?)?, utf8);
                    let depth = e.tags.get(&4).and_then(|v| v.first()).copied().unwrap_or(0);
                    Some((label, depth as usize, e.tags))
                })
                .collect()
        })
        .unwrap_or_default();

    let (parts, toc) = match version >= 8 {
        true => kf8(&text, &recs, &ncx, at(0xC0), at(0xFC), at(0xF8)).ok_or("corrupt KF8 index")?,
        false => mobi6(&text, &ncx),
    };

    let mut files = HashMap::new();
    let mut spine = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let href = format!("/part{i:04}.html");
        files.insert(href.clone(), decode(part, utf8).into_bytes());
        spine.push(href);
    }
    let first = at(0x6C) as usize;
    for (k, rec) in recs.iter().enumerate().skip(first) {
        if is_image(rec) {
            files.insert(format!("/res/{:05}", k - first + 1), rec.to_vec());
        }
    }
    let cover = exth
        .get(&201)
        .and_then(|v| u32_at(v, 0))
        .map(|i| format!("/res/{:05}", i + 1))
        .filter(|h| files.contains_key(h));

    let text_of = |t: u32| exth.get(&t).map(|v| decode(v, utf8).trim().to_string());
    let full_name = || {
        let (o, l) = (at(0x54) as usize, at(0x58) as usize);
        r0.get(o..o.saturating_add(l)).map(|v| decode(v, utf8))
    };
    Ok(Unpacked {
        title: text_of(503).or_else(full_name).unwrap_or_default(),
        author: text_of(100).unwrap_or_default(),
        language: text_of(524).unwrap_or_default(),
        spine,
        toc,
        files,
        cover,
    })
}

/// KF8: splice each skeleton's fragments back in, then point `kindle:` URLs at
/// the parts and images.
fn kf8(
    text: &[u8],
    recs: &[&[u8]],
    ncx: &[(String, usize, Tags)],
    fdst: u32,
    skel: u32,
    frag: u32,
) -> Option<(Vec<Vec<u8>>, Vec<(String, usize, String)>)> {
    // Flow 0 is the XHTML; the others are CSS and SVG, which the whitelist
    // policy drops anyway.
    let end = recs
        .get(fdst as usize)
        .filter(|r| r.starts_with(b"FDST"))
        .and_then(|r| u32_at(r, 16))
        .map_or(text.len(), |e| (e as usize).min(text.len()));
    let text = &text[..end];

    let (skel, _) = read_index(recs, skel as usize)?;
    let (frag, _) = read_index(recs, frag as usize)?;
    // A fragment's ident is its insert position, written out in decimal.
    let inserts: Vec<usize> = frag
        .iter()
        .map(|f| std::str::from_utf8(&f.ident).ok()?.parse().ok())
        .collect::<Option<_>>()?;

    // (assembled part, its start and end in the raw text)
    let mut parts = Vec::new();
    let mut fi = 0;
    for s in &skel {
        let count = *s.tags.get(&1)?.first()? as usize;
        let (pos, len) = pair(s.tags.get(&6)?)?;
        let mut base = pos + len;
        let mut out = text.get(pos..base)?.to_vec();
        for (f, &insert) in frag.get(fi..fi + count)?.iter().zip(&inserts[fi..]) {
            let (_, flen) = pair(f.tags.get(&6)?)?;
            let at = insert.checked_sub(pos)?.min(out.len());
            out.splice(at..at, text.get(base..base + flen)?.iter().copied());
            base += flen;
        }
        fi += count;
        parts.push((out, pos, base));
    }

    // `kindle:pos:fid:F:off:O` is an offset into fragment F, both base 32.
    let locate = |fid: usize, off: usize| {
        let pos = inserts.get(fid)? + off;
        let file = parts.iter().position(|p| p.1 <= pos && pos < p.2)?;
        let id = id_before(&parts[file].0, pos - parts[file].1);
        Some(format!("part{file:04}.html{}", id.map(|i| format!("#{i}")).unwrap_or_default()))
    };
    let base32 = |s: &str| usize::from_str_radix(s.split('?').next()?, 32).ok();
    let link = |url: &str| {
        let (fid, off) = url.strip_prefix("kindle:pos:fid:")?.split_once(":off:")?;
        locate(base32(fid)?, base32(off)?)
    };
    let embed = |url: &str| Some(format!("res/{:05}", base32(url.strip_prefix("kindle:embed:")?)?));

    let out = parts
        .iter()
        .map(|(p, _, _)| {
            let p = rewrite_attr(p, b"href=", |v| Some(format!("href=\"{}\"", link(v)?)));
            rewrite_attr(&p, b"src=", |v| Some(format!("src=\"{}\"", embed(v)?)))
        })
        .collect();
    let toc = ncx
        .iter()
        .filter_map(|(label, depth, tags)| {
            let (fid, off) = pair(tags.get(&6)?)?;
            Some((label.clone(), *depth, format!("/{}", locate(fid, off)?)))
        })
        .collect();
    Some((out, toc))
}

/// MOBI 6: cut the stream at page breaks, drop an anchor at every `filepos`
/// target, and turn `filepos` and `recindex` into `href` and `src`.
fn mobi6(text: &[u8], ncx: &[(String, usize, Tags)]) -> (Vec<Vec<u8>>, Vec<(String, usize, String)>) {
    let ncx_pos = |tags: &Tags| tags.get(&1).and_then(|v| v.first()).map(|&p| p as usize);
    let mut targets: Vec<usize> = ncx.iter().filter_map(|(_, _, t)| ncx_pos(t)).collect();
    // Collecting, not rewriting: the closure keeps every value and changes nothing.
    rewrite_attr(text, b"filepos=", |v| {
        targets.extend(v.parse::<usize>().ok());
        None
    });
    targets.sort_unstable();
    targets.dedup();

    // A break with nothing before it (a cover, a run of breaks) would be a
    // blank page; only cut once the current part has something to show.
    let mut starts = vec![0];
    let mut from = 0;
    while let Some(j) = find(&text[from..], b"<mbp:pagebreak") {
        let at = from + j;
        if has_content(&text[*starts.last().unwrap()..at]) {
            starts.push(at);
        }
        from = at + 1;
    }
    // Nor a last page of `</body></html>` after a final break.
    if starts.len() > 1 && !has_content(&text[*starts.last().unwrap()..]) {
        starts.pop();
    }
    let part_of = |pos: usize| starts.partition_point(|&s| s <= pos) - 1;

    let mut parts = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(text.len());
        let raw = &text[s..e];
        let mut html = Vec::with_capacity(raw.len());
        let mut last = 0;
        for &t in targets.iter().filter(|&&t| s <= t && t < e) {
            let mut at = t - s;
            // Inside a tag, or right on one: hang the anchor after it.
            let lt = raw[at..].iter().position(|&c| c == b'<');
            if let Some(gt) = raw[at..].iter().position(|&c| c == b'>') {
                if lt == Some(0) || lt.is_none_or(|l| gt < l) {
                    at += gt + 1;
                }
            }
            let at = at.max(last);
            html.extend_from_slice(&raw[last..at]);
            html.extend_from_slice(format!("<a id=\"fp{t}\"></a>").as_bytes());
            last = at;
        }
        html.extend_from_slice(&raw[last..]);
        let html = rewrite_attr(&html, b"filepos=", |v| {
            let p: usize = v.parse().ok()?;
            Some(format!("href=\"part{:04}.html#fp{p}\"", part_of(p)))
        });
        parts.push(rewrite_attr(&html, b"recindex=", |v| {
            Some(format!("src=\"res/{:05}\"", v.parse::<u32>().ok()?))
        }));
    }
    let toc = ncx
        .iter()
        .filter_map(|(label, depth, tags)| {
            let p = ncx_pos(tags)?;
            Some((label.clone(), *depth, format!("/part{:04}.html#fp{p}", part_of(p))))
        })
        .collect();
    (parts, toc)
}

/// Rewrite every `name=value` attribute. `f` gets the value and returns the
/// whole replacement attribute, or `None` to leave it as it is.
fn rewrite_attr(html: &[u8], name: &[u8], mut f: impl FnMut(&str) -> Option<String>) -> Vec<u8> {
    let mut out = Vec::with_capacity(html.len());
    let mut i = 0;
    while let Some(at) = find(&html[i..], name).map(|j| i + j) {
        let v = at + name.len();
        let quote = html.get(v).copied().filter(|c| matches!(c, b'"' | b'\''));
        let vs = v + quote.is_some() as usize;
        let ve = html[vs.min(html.len())..]
            .iter()
            .position(|&c| match quote {
                Some(q) => c == q,
                None => c.is_ascii_whitespace() || c == b'>' || c == b'/',
            })
            .map_or(html.len(), |e| vs + e);
        // A whole attribute, not the tail of `hirecindex=` or of prose.
        let whole = at > 0 && html[at - 1].is_ascii_whitespace();
        let new = whole
            .then(|| std::str::from_utf8(&html[vs.min(ve)..ve]).ok())
            .flatten()
            .and_then(&mut f);
        out.extend_from_slice(&html[i..at]);
        match new {
            Some(n) => {
                out.extend_from_slice(n.as_bytes());
                i = (ve + quote.is_some() as usize).min(html.len());
            }
            None => {
                out.extend_from_slice(&html[at..v]);
                i = v;
            }
        }
    }
    out.extend_from_slice(&html[i..]);
    out
}

/// The last `id` at or before `pos` in a part — a KF8 link names a byte
/// offset, and the nearest id is the closest thing a fragment can say.
fn id_before(part: &[u8], pos: usize) -> Option<String> {
    let mut end = pos.min(part.len());
    let lt = part[end..].iter().position(|&c| c == b'<');
    if let Some(gt) = part[end..].iter().position(|&c| c == b'>') {
        if lt == Some(0) || lt.is_none_or(|l| gt < l) {
            end += gt + 1;
        }
    }
    let head = &part[..end];
    let i = head.windows(4).rposition(|w| w == b" id=")?;
    let q = *part.get(i + 4).filter(|c| matches!(c, b'"' | b'\''))?;
    let rest = &part[i + 5..];
    let len = rest.iter().position(|&c| c == q)?;
    String::from_utf8(rest[..len].to_vec()).ok()
}

fn has_content(html: &[u8]) -> bool {
    let mut tag = false;
    find(html, b"<img").is_some()
        || html.iter().any(|&c| {
            match c {
                b'<' => tag = true,
                b'>' => tag = false,
                c if !tag && !c.is_ascii_whitespace() => return true,
                _ => {}
            }
            false
        })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn is_image(rec: &[u8]) -> bool {
    [&b"\xFF\xD8\xFF"[..], b"\x89PNG", b"GIF8", b"BM"].iter().any(|m| rec.starts_with(m))
}

fn decode(b: &[u8], utf8: bool) -> String {
    if utf8 {
        return String::from_utf8_lossy(b).into_owned();
    }
    // Windows-1252: Latin-1 except for 0x80..0xA0.
    const HI: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{9D}', 'ž', 'Ÿ',
    ];
    b.iter()
        .map(|&c| match c {
            0x80..0xA0 => HI[(c - 0x80) as usize],
            c => c as char,
        })
        .collect()
}

// --- PalmDB ---------------------------------------------------------------

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn records(data: &[u8]) -> Option<Vec<&[u8]>> {
    let n = u16_at(data, 76)? as usize;
    let offs: Vec<usize> =
        (0..n).map(|i| u32_at(data, 78 + 8 * i).map(|o| o as usize)).collect::<Option<_>>()?;
    (0..n)
        .map(|i| data.get(offs[i]..offs.get(i + 1).copied().unwrap_or(data.len())))
        .collect()
}

/// Bytes of trailing entries at the end of a text record, per the header's
/// extra-data flags. Bit 0 is the multibyte overlap; each higher bit is an
/// entry sized by a backwards varint.
fn trailing(rec: &[u8], flags: u16) -> usize {
    let mut n = 0;
    let mut f = flags >> 1;
    while f != 0 {
        if f & 1 != 0 {
            let mut size = 0;
            let mut shift = 0;
            for &b in rec[..rec.len().saturating_sub(n)].iter().rev() {
                size |= ((b & 0x7F) as usize) << shift;
                shift += 7;
                if b & 0x80 != 0 || shift >= 28 {
                    break;
                }
            }
            n += size;
        }
        f >>= 1;
    }
    if flags & 1 != 0 {
        if let Some(&b) = rec.len().checked_sub(n + 1).and_then(|i| rec.get(i)) {
            n += (b & 3) as usize + 1;
        }
    }
    n.min(rec.len())
}

// --- Compression ----------------------------------------------------------

/// PalmDoc LZ77, one record at a time: back-references never cross a record.
fn palmdoc(src: &[u8], out: &mut Vec<u8>) {
    let start = out.len();
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        i += 1;
        match c {
            1..=8 => {
                let end = (i + c as usize).min(src.len());
                out.extend_from_slice(&src[i..end]);
                i = end;
            }
            0 | 9..=0x7F => out.push(c),
            0xC0.. => out.extend_from_slice(&[b' ', c ^ 0x80]),
            _ => {
                let Some(&next) = src.get(i) else { break };
                i += 1;
                let pair = (c as usize) << 8 | next as usize;
                let (dist, len) = ((pair >> 3) & 0x7FF, (pair & 7) + 3);
                if dist == 0 || dist > out.len() - start {
                    continue;
                }
                for _ in 0..len {
                    out.push(out[out.len() - dist]);
                }
            }
        }
    }
}

/// HUFF/CDIC: a canonical Huffman code over a phrase dictionary whose entries
/// may themselves be compressed.
struct Huff {
    /// By the top byte of the code: (code length, terminal, max code).
    dict1: Vec<(usize, bool, u64)>,
    mincode: [u64; 33],
    maxcode: [u64; 33],
    /// (phrase, already expanded). `None` while being expanded, so a cycle in a
    /// hostile file fails instead of recursing forever.
    dict: Vec<Option<(Vec<u8>, bool)>>,
}

impl Huff {
    fn new(recs: &[&[u8]]) -> Option<Self> {
        let (huff, cdics) = recs.split_first()?;
        if !huff.starts_with(b"HUFF\0\0\0\x18") {
            return None;
        }
        let (off1, off2) = (u32_at(huff, 8)? as usize, u32_at(huff, 12)? as usize);
        let dict1 = (0..256)
            .map(|i| {
                let v = u32_at(huff, off1 + 4 * i)?;
                let len = (v & 0x1F) as usize;
                let max = (((v >> 8) as u64 + 1) << (32 - len)) - 1;
                (len != 0).then_some((len, v & 0x80 != 0, max))
            })
            .collect::<Option<_>>()?;
        let mut mincode = [0; 33];
        let mut maxcode = [u32::MAX as u64; 33];
        for k in 1..=32 {
            let lo = u32_at(huff, off2 + 8 * (k - 1))? as u64;
            let hi = u32_at(huff, off2 + 8 * (k - 1) + 4)? as u64;
            mincode[k] = lo << (32 - k);
            maxcode[k] = ((hi + 1) << (32 - k)) - 1;
        }
        let mut dict = Vec::new();
        for cdic in cdics {
            if !cdic.starts_with(b"CDIC\0\0\0\x10") {
                return None;
            }
            let phrases = u32_at(cdic, 8)? as usize;
            let bits = u32_at(cdic, 12)?.min(31);
            let n = (1usize << bits).min(phrases.saturating_sub(dict.len()));
            for i in 0..n {
                let off = u16_at(cdic, 16 + 2 * i)? as usize;
                let blen = u16_at(cdic, 16 + off)?;
                let phrase = cdic.get(18 + off..18 + off + (blen & 0x7FFF) as usize)?;
                dict.push(Some((phrase.to_vec(), blen & 0x8000 != 0)));
            }
        }
        Some(Self { dict1, mincode, maxcode, dict })
    }

    fn unpack(&mut self, data: &[u8], out: &mut Vec<u8>, depth: u32) -> Option<()> {
        if depth > 32 {
            return None;
        }
        let mut src = data.to_vec();
        src.extend_from_slice(&[0; 16]);
        let word = |p: usize| Some(u64::from_be_bytes(src.get(p..p + 8)?.try_into().ok()?));
        let mut bits_left = data.len() as i64 * 8;
        let mut pos = 0;
        let mut n: i64 = 32;
        let mut x = word(0)?;
        loop {
            if n <= 0 {
                pos += 4;
                x = word(pos)?;
                n += 32;
            }
            let code = (x >> n) & 0xFFFF_FFFF;
            let (mut len, term, mut max) = self.dict1[(code >> 24) as usize];
            if !term {
                while len < 32 && code < self.mincode[len] {
                    len += 1;
                }
                max = self.maxcode[len];
            }
            n -= len as i64;
            bits_left -= len as i64;
            if bits_left < 0 {
                return Some(());
            }
            let r = (max.checked_sub(code)? >> (32 - len)) as usize;
            let (phrase, done) = self.dict.get_mut(r)?.take()?;
            let phrase = match done {
                true => phrase,
                false => {
                    let mut expanded = Vec::new();
                    self.unpack(&phrase, &mut expanded, depth + 1)?;
                    expanded
                }
            };
            out.extend_from_slice(&phrase);
            self.dict[r] = Some((phrase, true));
        }
    }
}

// --- INDX -----------------------------------------------------------------

type Tags = HashMap<u8, Vec<u32>>;

struct Entry {
    ident: Vec<u8>,
    tags: Tags,
}

fn pair(v: &[u32]) -> Option<(usize, usize)> {
    Some((*v.first()? as usize, *v.get(1)? as usize))
}

/// Forward varint: 7 bits a byte, the last byte flagged by its high bit.
fn decint(b: &[u8]) -> Option<(u32, usize)> {
    let mut v = 0u32;
    for (i, &x) in b.iter().enumerate() {
        v = (v << 7) | (x & 0x7F) as u32;
        if x & 0x80 != 0 {
            return Some((v, i + 1));
        }
    }
    None
}

/// An INDX table: its entries, and the CNCX records their strings live in.
fn read_index<'a>(recs: &[&'a [u8]], idx: usize) -> Option<(Vec<Entry>, Vec<&'a [u8]>)> {
    let head = *recs.get(idx)?;
    if !head.starts_with(b"INDX") {
        return None;
    }
    let count = u32_at(head, 24)? as usize;
    let ncncx = u32_at(head, 52)? as usize;
    let tagx = head.get(u32_at(head, 4)? as usize..)?;
    if !tagx.starts_with(b"TAGX") {
        return None;
    }
    let control = u32_at(tagx, 8)? as usize;
    let tagx: Vec<&[u8]> = tagx.get(12..u32_at(tagx, 4)? as usize)?.chunks_exact(4).collect();
    let cncx = recs.get(idx + 1 + count..idx + 1 + count + ncncx)?.to_vec();

    let mut out = Vec::new();
    for rec in recs.get(idx + 1..idx + 1 + count)? {
        let idxt = u32_at(rec, 20)? as usize;
        let mut pos: Vec<usize> = (0..u32_at(rec, 24)? as usize)
            .map(|j| u16_at(rec, idxt + 4 + 2 * j).map(usize::from))
            .collect::<Option<_>>()?;
        pos.push(idxt);
        for w in pos.windows(2) {
            let e = rec.get(w[0]..w[1])?;
            let len = *e.first()? as usize;
            out.push(Entry {
                ident: e.get(1..1 + len)?.to_vec(),
                tags: tag_map(&tagx, control, e.get(1 + len..)?)?,
            });
        }
    }
    Some((out, cncx))
}

/// Decode one entry's tags against the TAGX table (tag, values per entry,
/// bitmask, end-of-control-byte).
fn tag_map(tagx: &[&[u8]], control: usize, data: &[u8]) -> Option<Tags> {
    enum Len {
        Values(usize),
        Bytes(usize),
    }
    let mut ci = 0;
    let mut rest = data.get(control..)?;
    let mut present = Vec::new();
    for t in tagx {
        let (tag, per, mask, eof) = (t[0], t[1] as usize, t[2], t[3]);
        if eof == 1 {
            ci += 1;
            continue;
        }
        let v = *data.get(ci)? & mask;
        if v == 0 {
            continue;
        }
        if v == mask && mask.count_ones() > 1 {
            let (n, used) = decint(rest)?;
            rest = &rest[used..];
            present.push((tag, per, Len::Bytes(n as usize)));
        } else {
            present.push((tag, per, Len::Values((v >> mask.trailing_zeros()) as usize)));
        }
    }
    let mut tags = HashMap::new();
    for (tag, per, len) in present {
        let mut vals = Vec::new();
        let mut take = || {
            let (v, used) = decint(rest)?;
            rest = &rest[used..];
            vals.push(v);
            Some(used)
        };
        match len {
            Len::Values(n) => {
                for _ in 0..n * per {
                    take()?;
                }
            }
            Len::Bytes(n) => {
                let mut used = 0;
                while used < n {
                    used += take()?;
                }
            }
        }
        tags.insert(tag, vals);
    }
    Some(tags)
}

/// A string in the CNCX records: the offset's high half picks the record.
fn cncx_str<'a>(cncx: &[&'a [u8]], off: u32) -> Option<&'a [u8]> {
    let rest = cncx.get((off >> 16) as usize)?.get((off & 0xFFFF) as usize..)?;
    let (len, used) = decint(rest)?;
    rest.get(used..used + len as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MOBI 6 file with uncompressed text: a header, one text record and one
    /// image, which is all the container needs to be.
    fn tiny_mobi6(text: &[u8]) -> Vec<u8> {
        let mut r0 = vec![0u8; 16 + 0xE8];
        r0[0..2].copy_from_slice(&1u16.to_be_bytes()); // no compression
        r0[8..10].copy_from_slice(&1u16.to_be_bytes()); // one text record
        r0[16..20].copy_from_slice(b"MOBI");
        r0[20..24].copy_from_slice(&0xE8u32.to_be_bytes());
        r0[28..32].copy_from_slice(&65001u32.to_be_bytes());
        r0[36..40].copy_from_slice(&6u32.to_be_bytes());
        r0[0x6C..0x70].copy_from_slice(&2u32.to_be_bytes()); // first image
        r0[0xF4..0xF8].copy_from_slice(&NULL.to_be_bytes()); // no NCX
        let recs = [r0, text.to_vec(), b"\x89PNG fake".to_vec()];

        let mut out = vec![0u8; 78 + 8 * recs.len() + 2];
        out[60..68].copy_from_slice(b"BOOKMOBI");
        out[76..78].copy_from_slice(&(recs.len() as u16).to_be_bytes());
        for (i, r) in recs.iter().enumerate() {
            let off = out.len() as u32;
            out[78 + 8 * i..82 + 8 * i].copy_from_slice(&off.to_be_bytes());
            out.extend_from_slice(r);
        }
        out
    }

    #[test]
    fn mobi6_splits_at_page_breaks_and_resolves_links_and_images() {
        let text = b"<html><body><p><a filepos=0000000081>to two</a></p>\
                     <mbp:pagebreak/><mbp:pagebreak/><p>Two <img recindex=\"00001\"></p><mbp:pagebreak/></body></html>";
        let target = find(text, b"<mbp:pagebreak/><p>").unwrap();
        let text = String::from_utf8_lossy(text).replace("0000000081", &format!("{target:010}"));
        let book = unpack(&tiny_mobi6(text.as_bytes())).unwrap();

        assert_eq!(book.spine, ["/part0000.html", "/part0001.html"], "empty parts merged");
        let p0 = String::from_utf8(book.files["/part0000.html"].clone()).unwrap();
        let p1 = String::from_utf8(book.files["/part0001.html"].clone()).unwrap();
        assert!(p0.contains(&format!("href=\"part0001.html#fp{target}\"")), "{p0}");
        assert!(p1.contains(&format!("<a id=\"fp{target}\"></a>")), "{p1}");
        assert!(p1.contains("src=\"res/00001\""), "{p1}");
        assert!(book.files.contains_key("/res/00001"));
    }

    #[test]
    fn palmdoc_literals_spaces_and_back_references() {
        let mut out = Vec::new();
        // "ab", then space+'c', then copy 2 back for 3 bytes.
        palmdoc(&[b'a', b'b', b'c' ^ 0x80, 0x80, (2 << 3) as u8], &mut out);
        assert_eq!(out, b"ab c c ");
    }

    #[test]
    fn trailing_entries_are_stripped() {
        // Text, one overlap byte and its count, then a 2-byte entry that ends in
        // its own size.
        let rec = [b'h', b'i', b'X', 0x01, b'e', 0x82];
        assert_eq!(trailing(&rec, 0b11), 4);
    }

    #[test]
    fn rewrite_leaves_lookalikes_alone() {
        let html = b"<img hirecindex=\"2\" recindex=\"3\">";
        let out = rewrite_attr(html, b"recindex=", |v| Some(format!("src=\"{v}\"")));
        assert_eq!(out, b"<img hirecindex=\"2\" src=\"3\">");
    }
}

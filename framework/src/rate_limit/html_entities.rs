//! PHP's `htmlentities` under its default flags: the first step of
//! Laravel's `RateLimiter::cleanRateLimiterKey`.
//!
//! Laravel cleans a key with
//! `preg_replace('/&([a-z])[a-z]+;/i', '$1', htmlentities($key))`. The
//! strip alone does not write the key Laravel writes: `htmlentities` first
//! turns `é` into `&eacute;`, which the strip then reduces to `e`, so
//! `café` and `cafe` count in one bucket. This module holds the encoding
//! step, so [`RateLimiter::clean_rate_limiter_key`](super::RateLimiter::clean_rate_limiter_key)
//! writes the key Laravel writes.
//!
//! Since PHP 8.1, `htmlentities` defaults to
//! `ENT_QUOTES | ENT_SUBSTITUTE | ENT_HTML401` and UTF-8. Under those flags
//! every character with a named HTML 4.01 entity becomes that entity, `'`
//! becomes `&#039;` (HTML 4.01 names no apostrophe), and every other
//! character stays as it is. An entity already in the input is encoded
//! again (`double_encode` defaults to true), so `&amp;` becomes `&amp;amp;`.
//! `ENT_SUBSTITUTE` only acts on invalid UTF-8, which a `&str` cannot hold.

/// The table `htmlentities` reads under `ENT_QUOTES | ENT_HTML401` and
/// UTF-8: the 252 named HTML 4.01 entities and `&#039;`, as PHP's
/// `get_html_translation_table(HTML_ENTITIES, ENT_QUOTES | ENT_HTML401, 'UTF-8')`
/// returns them. Sorted by character, so a lookup is a binary search. The
/// names carry no `&` or `;`. Characters are written as escapes, so the
/// invisible ones (no-break space, the zero-width joiners) stay readable.
const HTML401_ENTITIES: [(char, &str); 253] = [
    ('\u{22}', "quot"),
    ('\u{26}', "amp"),
    ('\u{27}', "#039"),
    ('\u{3c}', "lt"),
    ('\u{3e}', "gt"),
    ('\u{a0}', "nbsp"),
    ('\u{a1}', "iexcl"),
    ('\u{a2}', "cent"),
    ('\u{a3}', "pound"),
    ('\u{a4}', "curren"),
    ('\u{a5}', "yen"),
    ('\u{a6}', "brvbar"),
    ('\u{a7}', "sect"),
    ('\u{a8}', "uml"),
    ('\u{a9}', "copy"),
    ('\u{aa}', "ordf"),
    ('\u{ab}', "laquo"),
    ('\u{ac}', "not"),
    ('\u{ad}', "shy"),
    ('\u{ae}', "reg"),
    ('\u{af}', "macr"),
    ('\u{b0}', "deg"),
    ('\u{b1}', "plusmn"),
    ('\u{b2}', "sup2"),
    ('\u{b3}', "sup3"),
    ('\u{b4}', "acute"),
    ('\u{b5}', "micro"),
    ('\u{b6}', "para"),
    ('\u{b7}', "middot"),
    ('\u{b8}', "cedil"),
    ('\u{b9}', "sup1"),
    ('\u{ba}', "ordm"),
    ('\u{bb}', "raquo"),
    ('\u{bc}', "frac14"),
    ('\u{bd}', "frac12"),
    ('\u{be}', "frac34"),
    ('\u{bf}', "iquest"),
    ('\u{c0}', "Agrave"),
    ('\u{c1}', "Aacute"),
    ('\u{c2}', "Acirc"),
    ('\u{c3}', "Atilde"),
    ('\u{c4}', "Auml"),
    ('\u{c5}', "Aring"),
    ('\u{c6}', "AElig"),
    ('\u{c7}', "Ccedil"),
    ('\u{c8}', "Egrave"),
    ('\u{c9}', "Eacute"),
    ('\u{ca}', "Ecirc"),
    ('\u{cb}', "Euml"),
    ('\u{cc}', "Igrave"),
    ('\u{cd}', "Iacute"),
    ('\u{ce}', "Icirc"),
    ('\u{cf}', "Iuml"),
    ('\u{d0}', "ETH"),
    ('\u{d1}', "Ntilde"),
    ('\u{d2}', "Ograve"),
    ('\u{d3}', "Oacute"),
    ('\u{d4}', "Ocirc"),
    ('\u{d5}', "Otilde"),
    ('\u{d6}', "Ouml"),
    ('\u{d7}', "times"),
    ('\u{d8}', "Oslash"),
    ('\u{d9}', "Ugrave"),
    ('\u{da}', "Uacute"),
    ('\u{db}', "Ucirc"),
    ('\u{dc}', "Uuml"),
    ('\u{dd}', "Yacute"),
    ('\u{de}', "THORN"),
    ('\u{df}', "szlig"),
    ('\u{e0}', "agrave"),
    ('\u{e1}', "aacute"),
    ('\u{e2}', "acirc"),
    ('\u{e3}', "atilde"),
    ('\u{e4}', "auml"),
    ('\u{e5}', "aring"),
    ('\u{e6}', "aelig"),
    ('\u{e7}', "ccedil"),
    ('\u{e8}', "egrave"),
    ('\u{e9}', "eacute"),
    ('\u{ea}', "ecirc"),
    ('\u{eb}', "euml"),
    ('\u{ec}', "igrave"),
    ('\u{ed}', "iacute"),
    ('\u{ee}', "icirc"),
    ('\u{ef}', "iuml"),
    ('\u{f0}', "eth"),
    ('\u{f1}', "ntilde"),
    ('\u{f2}', "ograve"),
    ('\u{f3}', "oacute"),
    ('\u{f4}', "ocirc"),
    ('\u{f5}', "otilde"),
    ('\u{f6}', "ouml"),
    ('\u{f7}', "divide"),
    ('\u{f8}', "oslash"),
    ('\u{f9}', "ugrave"),
    ('\u{fa}', "uacute"),
    ('\u{fb}', "ucirc"),
    ('\u{fc}', "uuml"),
    ('\u{fd}', "yacute"),
    ('\u{fe}', "thorn"),
    ('\u{ff}', "yuml"),
    ('\u{152}', "OElig"),
    ('\u{153}', "oelig"),
    ('\u{160}', "Scaron"),
    ('\u{161}', "scaron"),
    ('\u{178}', "Yuml"),
    ('\u{192}', "fnof"),
    ('\u{2c6}', "circ"),
    ('\u{2dc}', "tilde"),
    ('\u{391}', "Alpha"),
    ('\u{392}', "Beta"),
    ('\u{393}', "Gamma"),
    ('\u{394}', "Delta"),
    ('\u{395}', "Epsilon"),
    ('\u{396}', "Zeta"),
    ('\u{397}', "Eta"),
    ('\u{398}', "Theta"),
    ('\u{399}', "Iota"),
    ('\u{39a}', "Kappa"),
    ('\u{39b}', "Lambda"),
    ('\u{39c}', "Mu"),
    ('\u{39d}', "Nu"),
    ('\u{39e}', "Xi"),
    ('\u{39f}', "Omicron"),
    ('\u{3a0}', "Pi"),
    ('\u{3a1}', "Rho"),
    ('\u{3a3}', "Sigma"),
    ('\u{3a4}', "Tau"),
    ('\u{3a5}', "Upsilon"),
    ('\u{3a6}', "Phi"),
    ('\u{3a7}', "Chi"),
    ('\u{3a8}', "Psi"),
    ('\u{3a9}', "Omega"),
    ('\u{3b1}', "alpha"),
    ('\u{3b2}', "beta"),
    ('\u{3b3}', "gamma"),
    ('\u{3b4}', "delta"),
    ('\u{3b5}', "epsilon"),
    ('\u{3b6}', "zeta"),
    ('\u{3b7}', "eta"),
    ('\u{3b8}', "theta"),
    ('\u{3b9}', "iota"),
    ('\u{3ba}', "kappa"),
    ('\u{3bb}', "lambda"),
    ('\u{3bc}', "mu"),
    ('\u{3bd}', "nu"),
    ('\u{3be}', "xi"),
    ('\u{3bf}', "omicron"),
    ('\u{3c0}', "pi"),
    ('\u{3c1}', "rho"),
    ('\u{3c2}', "sigmaf"),
    ('\u{3c3}', "sigma"),
    ('\u{3c4}', "tau"),
    ('\u{3c5}', "upsilon"),
    ('\u{3c6}', "phi"),
    ('\u{3c7}', "chi"),
    ('\u{3c8}', "psi"),
    ('\u{3c9}', "omega"),
    ('\u{3d1}', "thetasym"),
    ('\u{3d2}', "upsih"),
    ('\u{3d6}', "piv"),
    ('\u{2002}', "ensp"),
    ('\u{2003}', "emsp"),
    ('\u{2009}', "thinsp"),
    ('\u{200c}', "zwnj"),
    ('\u{200d}', "zwj"),
    ('\u{200e}', "lrm"),
    ('\u{200f}', "rlm"),
    ('\u{2013}', "ndash"),
    ('\u{2014}', "mdash"),
    ('\u{2018}', "lsquo"),
    ('\u{2019}', "rsquo"),
    ('\u{201a}', "sbquo"),
    ('\u{201c}', "ldquo"),
    ('\u{201d}', "rdquo"),
    ('\u{201e}', "bdquo"),
    ('\u{2020}', "dagger"),
    ('\u{2021}', "Dagger"),
    ('\u{2022}', "bull"),
    ('\u{2026}', "hellip"),
    ('\u{2030}', "permil"),
    ('\u{2032}', "prime"),
    ('\u{2033}', "Prime"),
    ('\u{2039}', "lsaquo"),
    ('\u{203a}', "rsaquo"),
    ('\u{203e}', "oline"),
    ('\u{2044}', "frasl"),
    ('\u{20ac}', "euro"),
    ('\u{2111}', "image"),
    ('\u{2118}', "weierp"),
    ('\u{211c}', "real"),
    ('\u{2122}', "trade"),
    ('\u{2135}', "alefsym"),
    ('\u{2190}', "larr"),
    ('\u{2191}', "uarr"),
    ('\u{2192}', "rarr"),
    ('\u{2193}', "darr"),
    ('\u{2194}', "harr"),
    ('\u{21b5}', "crarr"),
    ('\u{21d0}', "lArr"),
    ('\u{21d1}', "uArr"),
    ('\u{21d2}', "rArr"),
    ('\u{21d3}', "dArr"),
    ('\u{21d4}', "hArr"),
    ('\u{2200}', "forall"),
    ('\u{2202}', "part"),
    ('\u{2203}', "exist"),
    ('\u{2205}', "empty"),
    ('\u{2207}', "nabla"),
    ('\u{2208}', "isin"),
    ('\u{2209}', "notin"),
    ('\u{220b}', "ni"),
    ('\u{220f}', "prod"),
    ('\u{2211}', "sum"),
    ('\u{2212}', "minus"),
    ('\u{2217}', "lowast"),
    ('\u{221a}', "radic"),
    ('\u{221d}', "prop"),
    ('\u{221e}', "infin"),
    ('\u{2220}', "ang"),
    ('\u{2227}', "and"),
    ('\u{2228}', "or"),
    ('\u{2229}', "cap"),
    ('\u{222a}', "cup"),
    ('\u{222b}', "int"),
    ('\u{2234}', "there4"),
    ('\u{223c}', "sim"),
    ('\u{2245}', "cong"),
    ('\u{2248}', "asymp"),
    ('\u{2260}', "ne"),
    ('\u{2261}', "equiv"),
    ('\u{2264}', "le"),
    ('\u{2265}', "ge"),
    ('\u{2282}', "sub"),
    ('\u{2283}', "sup"),
    ('\u{2284}', "nsub"),
    ('\u{2286}', "sube"),
    ('\u{2287}', "supe"),
    ('\u{2295}', "oplus"),
    ('\u{2297}', "otimes"),
    ('\u{22a5}', "perp"),
    ('\u{22c5}', "sdot"),
    ('\u{2308}', "lceil"),
    ('\u{2309}', "rceil"),
    ('\u{230a}', "lfloor"),
    ('\u{230b}', "rfloor"),
    ('\u{2329}', "lang"),
    ('\u{232a}', "rang"),
    ('\u{25ca}', "loz"),
    ('\u{2660}', "spades"),
    ('\u{2663}', "clubs"),
    ('\u{2665}', "hearts"),
    ('\u{2666}', "diams"),
];

/// Encode `input` as PHP's `htmlentities($input)` does with its default
/// flags and UTF-8.
pub(super) fn htmlentities(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for character in input.chars() {
        match HTML401_ENTITIES.binary_search_by_key(&character, |&(entity_char, _)| entity_char) {
            Ok(index) => {
                encoded.push('&');
                encoded.push_str(HTML401_ENTITIES[index].1);
                encoded.push(';');
            }
            Err(_) => encoded.push(character),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_names_each_character_once() {
        assert!(
            HTML401_ENTITIES
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "the binary search needs the table sorted by character, without repeats"
        );
    }

    #[test]
    fn encodes_named_entities_and_the_apostrophe_as_php_does() {
        assert_eq!(htmlentities("café"), "caf&eacute;");
        assert_eq!(htmlentities("a&b"), "a&amp;b");
        assert_eq!(htmlentities("<\"it's\">"), "&lt;&quot;it&#039;s&quot;&gt;");
        assert_eq!(htmlentities("x\u{a0}y"), "x&nbsp;y");
        assert_eq!(htmlentities("\u{2013}"), "&ndash;");
        assert_eq!(htmlentities("x²"), "x&sup2;");
    }

    #[test]
    fn leaves_characters_without_an_entity_alone() {
        assert_eq!(htmlentities("user:42"), "user:42");
        assert_eq!(htmlentities("日本"), "日本");
        assert_eq!(htmlentities(""), "");
    }

    #[test]
    fn encodes_an_entity_already_in_the_input_again() {
        assert_eq!(htmlentities("&amp;"), "&amp;amp;");
    }
}

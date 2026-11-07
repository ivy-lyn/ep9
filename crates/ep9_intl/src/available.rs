//! Available locales (ECMA-402 §9.1) and the lists of Intl.supportedValuesOf (8.3.2).
//!
//! ```text
//! CLDR_LOCALES (CLDR 48.2.1 availableLocales "full", 766 ids)
//!   + the default-content children that likely subtags give (en -> en-US; zh-Hant ->
//!     zh-Hant-TW and zh-TW; sr-Cyrl-BA -> sr-BA)          locales(): a sorted Vec, OnceLock
//! supportedValuesOf: CALENDARS, COLLATIONS, CURRENCIES, NUMBERING_SYSTEMS, UNITS (static,
//!   sorted), time zones from ep9_temporal::tzdb::zone_ids() (INTL-I1-D9)
//! ```
//!
//! The list is static, not read from the data tries: the baked data is deduplicated, so a
//! child such as `en-US` has no entry of its own, and `resolvedOptions().locale` must still
//! say `en-US` (INTL-I1 §4.5). A locale of the list that the compiled data does not have
//! (the data is CLDR "recommended") formats with its fallback data. One list serves every W1
//! service; a later service can filter it by its own data.

use std::sync::OnceLock;

use icu_locale::LocaleExpander;
use icu_locale_core::LanguageIdentifier;

/// The CLDR 48.2.1 `availableLocales.json` "full" ids (the CLDR release of the ICU4X 2.3
/// compiled data), sorted. Canonical tags.
pub const CLDR_LOCALES: &[&str] = &[
    "aa",
    "aa-DJ",
    "aa-ER",
    "ab",
    "af",
    "af-NA",
    "agq",
    "ak",
    "am",
    "an",
    "ann",
    "apc",
    "ar",
    "ar-AE",
    "ar-BH",
    "ar-DJ",
    "ar-DZ",
    "ar-EG",
    "ar-EH",
    "ar-ER",
    "ar-IL",
    "ar-IQ",
    "ar-JO",
    "ar-KM",
    "ar-KW",
    "ar-LB",
    "ar-LY",
    "ar-MA",
    "ar-MR",
    "ar-OM",
    "ar-PS",
    "ar-QA",
    "ar-SA",
    "ar-SD",
    "ar-SO",
    "ar-SS",
    "ar-SY",
    "ar-TD",
    "ar-TN",
    "ar-YE",
    "arn",
    "as",
    "asa",
    "ast",
    "az",
    "az-Arab",
    "az-Arab-IQ",
    "az-Arab-TR",
    "az-Cyrl",
    "az-Latn",
    "ba",
    "bal",
    "bal-Arab",
    "bal-Latn",
    "bas",
    "be",
    "be-tarask",
    "bem",
    "bew",
    "bez",
    "bg",
    "bgc",
    "bgn",
    "bgn-AE",
    "bgn-AF",
    "bgn-IR",
    "bgn-OM",
    "bho",
    "blo",
    "blt",
    "bm",
    "bm-Nkoo",
    "bn",
    "bn-IN",
    "bo",
    "bo-IN",
    "bqi",
    "br",
    "brx",
    "bs",
    "bs-Cyrl",
    "bs-Latn",
    "bss",
    "bua",
    "byn",
    "ca",
    "ca-AD",
    "ca-ES-valencia",
    "ca-FR",
    "ca-IT",
    "cad",
    "cch",
    "ccp",
    "ccp-IN",
    "ce",
    "ceb",
    "cgg",
    "cho",
    "chr",
    "cic",
    "ckb",
    "ckb-IR",
    "co",
    "cop",
    "cs",
    "csw",
    "cu",
    "cv",
    "cy",
    "da",
    "da-GL",
    "dav",
    "de",
    "de-AT",
    "de-BE",
    "de-CH",
    "de-IT",
    "de-LI",
    "de-LU",
    "dje",
    "doi",
    "dsb",
    "dua",
    "dv",
    "dyo",
    "dz",
    "ebu",
    "ee",
    "ee-TG",
    "el",
    "el-CY",
    "el-polyton",
    "en",
    "en-001",
    "en-150",
    "en-AE",
    "en-AG",
    "en-AI",
    "en-AS",
    "en-AT",
    "en-AU",
    "en-BB",
    "en-BE",
    "en-BI",
    "en-BM",
    "en-BS",
    "en-BW",
    "en-BZ",
    "en-CA",
    "en-CC",
    "en-CH",
    "en-CK",
    "en-CM",
    "en-CX",
    "en-CY",
    "en-CZ",
    "en-DE",
    "en-DG",
    "en-DK",
    "en-DM",
    "en-Dsrt",
    "en-EE",
    "en-ER",
    "en-ES",
    "en-FI",
    "en-FJ",
    "en-FK",
    "en-FM",
    "en-FR",
    "en-GB",
    "en-GD",
    "en-GE",
    "en-GG",
    "en-GH",
    "en-GI",
    "en-GM",
    "en-GS",
    "en-GU",
    "en-GY",
    "en-HK",
    "en-HU",
    "en-ID",
    "en-IE",
    "en-IL",
    "en-IM",
    "en-IN",
    "en-IO",
    "en-IT",
    "en-JE",
    "en-JM",
    "en-JP",
    "en-KE",
    "en-KI",
    "en-KN",
    "en-KY",
    "en-LC",
    "en-LR",
    "en-LS",
    "en-LT",
    "en-LV",
    "en-MG",
    "en-MH",
    "en-MO",
    "en-MP",
    "en-MS",
    "en-MT",
    "en-MU",
    "en-MV",
    "en-MW",
    "en-MY",
    "en-NA",
    "en-NF",
    "en-NG",
    "en-NL",
    "en-NO",
    "en-NR",
    "en-NU",
    "en-NZ",
    "en-PG",
    "en-PH",
    "en-PK",
    "en-PL",
    "en-PN",
    "en-PR",
    "en-PT",
    "en-PW",
    "en-RO",
    "en-RW",
    "en-SB",
    "en-SC",
    "en-SD",
    "en-SE",
    "en-SG",
    "en-SH",
    "en-SI",
    "en-SK",
    "en-SL",
    "en-SS",
    "en-SX",
    "en-SZ",
    "en-Shaw",
    "en-TC",
    "en-TK",
    "en-TO",
    "en-TT",
    "en-TV",
    "en-TZ",
    "en-UA",
    "en-UG",
    "en-UM",
    "en-VC",
    "en-VG",
    "en-VI",
    "en-VU",
    "en-WS",
    "en-ZA",
    "en-ZM",
    "en-ZW",
    "eo",
    "es",
    "es-419",
    "es-AR",
    "es-BO",
    "es-BR",
    "es-BZ",
    "es-CL",
    "es-CO",
    "es-CR",
    "es-CU",
    "es-DO",
    "es-EA",
    "es-EC",
    "es-GQ",
    "es-GT",
    "es-HN",
    "es-IC",
    "es-MX",
    "es-NI",
    "es-PA",
    "es-PE",
    "es-PH",
    "es-PR",
    "es-PY",
    "es-SV",
    "es-US",
    "es-UY",
    "es-VE",
    "et",
    "eu",
    "ewo",
    "fa",
    "fa-AF",
    "ff",
    "ff-Adlm",
    "ff-Adlm-BF",
    "ff-Adlm-CM",
    "ff-Adlm-GH",
    "ff-Adlm-GM",
    "ff-Adlm-GW",
    "ff-Adlm-LR",
    "ff-Adlm-MR",
    "ff-Adlm-NE",
    "ff-Adlm-NG",
    "ff-Adlm-SL",
    "ff-Adlm-SN",
    "ff-Latn",
    "ff-Latn-BF",
    "ff-Latn-CM",
    "ff-Latn-GH",
    "ff-Latn-GM",
    "ff-Latn-GN",
    "ff-Latn-GW",
    "ff-Latn-LR",
    "ff-Latn-MR",
    "ff-Latn-NE",
    "ff-Latn-NG",
    "ff-Latn-SL",
    "fi",
    "fil",
    "fo",
    "fo-DK",
    "fr",
    "fr-BE",
    "fr-BF",
    "fr-BI",
    "fr-BJ",
    "fr-BL",
    "fr-CA",
    "fr-CD",
    "fr-CF",
    "fr-CG",
    "fr-CH",
    "fr-CI",
    "fr-CM",
    "fr-DJ",
    "fr-DZ",
    "fr-GA",
    "fr-GF",
    "fr-GN",
    "fr-GP",
    "fr-GQ",
    "fr-HT",
    "fr-KM",
    "fr-LU",
    "fr-MA",
    "fr-MC",
    "fr-MF",
    "fr-MG",
    "fr-ML",
    "fr-MQ",
    "fr-MR",
    "fr-MU",
    "fr-NC",
    "fr-NE",
    "fr-PF",
    "fr-PM",
    "fr-RE",
    "fr-RW",
    "fr-SC",
    "fr-SN",
    "fr-SY",
    "fr-TD",
    "fr-TG",
    "fr-TN",
    "fr-VU",
    "fr-WF",
    "fr-YT",
    "frr",
    "fur",
    "fy",
    "ga",
    "ga-GB",
    "gaa",
    "gd",
    "gez",
    "gez-ER",
    "gl",
    "gn",
    "gsw",
    "gsw-FR",
    "gsw-LI",
    "gu",
    "guz",
    "gv",
    "ha",
    "ha-Arab",
    "ha-Arab-SD",
    "ha-GH",
    "ha-NE",
    "haw",
    "he",
    "hi",
    "hi-Latn",
    "hnj",
    "hnj-Hmnp",
    "hr",
    "hr-BA",
    "hsb",
    "ht",
    "hu",
    "hy",
    "ia",
    "id",
    "ie",
    "ig",
    "ii",
    "io",
    "is",
    "it",
    "it-CH",
    "it-SM",
    "it-VA",
    "iu",
    "iu-Latn",
    "ja",
    "jbo",
    "jgo",
    "jmc",
    "jv",
    "ka",
    "kaa",
    "kaa-Cyrl",
    "kaa-Latn",
    "kab",
    "kaj",
    "kam",
    "kcg",
    "kde",
    "kea",
    "kek",
    "ken",
    "kgp",
    "khq",
    "ki",
    "kk",
    "kk-Arab",
    "kk-Cyrl",
    "kk-KZ",
    "kkj",
    "kl",
    "kln",
    "km",
    "kn",
    "ko",
    "ko-CN",
    "ko-KP",
    "kok",
    "kok-Deva",
    "kok-Latn",
    "kpe",
    "kpe-GN",
    "ks",
    "ks-Arab",
    "ks-Deva",
    "ksb",
    "ksf",
    "ksh",
    "ku",
    "ku-Arab",
    "ku-Arab-IR",
    "ku-Latn",
    "ku-Latn-IQ",
    "ku-Latn-SY",
    "ku-TR",
    "kw",
    "kxv",
    "kxv-Deva",
    "kxv-Latn",
    "kxv-Orya",
    "kxv-Telu",
    "ky",
    "la",
    "lag",
    "lb",
    "lg",
    "lij",
    "lkt",
    "lld",
    "lmo",
    "ln",
    "ln-AO",
    "ln-CF",
    "ln-CG",
    "lo",
    "lrc",
    "lrc-IQ",
    "lt",
    "ltg",
    "lu",
    "luo",
    "luy",
    "lv",
    "lzz",
    "mai",
    "mas",
    "mas-TZ",
    "mdf",
    "mer",
    "mfe",
    "mg",
    "mgh",
    "mgo",
    "mhn",
    "mi",
    "mic",
    "mk",
    "ml",
    "mn",
    "mn-Mong",
    "mn-Mong-MN",
    "mni",
    "mni-Beng",
    "mni-Mtei",
    "moh",
    "mr",
    "ms",
    "ms-Arab",
    "ms-Arab-BN",
    "ms-BN",
    "ms-ID",
    "ms-SG",
    "mt",
    "mua",
    "mus",
    "mww",
    "mww-Hmnp",
    "my",
    "myv",
    "mzn",
    "naq",
    "nb",
    "nb-SJ",
    "nd",
    "nds",
    "nds-NL",
    "ne",
    "ne-IN",
    "nl",
    "nl-AW",
    "nl-BE",
    "nl-BQ",
    "nl-CW",
    "nl-SR",
    "nl-SX",
    "nmg",
    "nn",
    "nnh",
    "no",
    "nqo",
    "nr",
    "nso",
    "nus",
    "nv",
    "ny",
    "nyn",
    "oc",
    "oc-ES",
    "oka",
    "oka-US",
    "om",
    "om-KE",
    "or",
    "os",
    "os-RU",
    "osa",
    "pa",
    "pa-Arab",
    "pa-Guru",
    "pap",
    "pap-AW",
    "pcm",
    "pi",
    "pi-Latn",
    "pis",
    "pl",
    "pms",
    "prg",
    "ps",
    "ps-PK",
    "pt",
    "pt-AO",
    "pt-CH",
    "pt-CV",
    "pt-GQ",
    "pt-GW",
    "pt-LU",
    "pt-MO",
    "pt-MZ",
    "pt-PT",
    "pt-ST",
    "pt-TL",
    "qu",
    "qu-BO",
    "qu-EC",
    "quc",
    "raj",
    "rhg",
    "rhg-Rohg",
    "rhg-Rohg-BD",
    "rif",
    "rm",
    "rn",
    "ro",
    "ro-MD",
    "rof",
    "ru",
    "ru-BY",
    "ru-KG",
    "ru-KZ",
    "ru-MD",
    "ru-UA",
    "rw",
    "rwk",
    "sa",
    "sah",
    "saq",
    "sat",
    "sat-Deva",
    "sat-Olck",
    "sbp",
    "sc",
    "scn",
    "sd",
    "sd-Arab",
    "sd-Deva",
    "sdh",
    "sdh-IQ",
    "se",
    "se-FI",
    "se-SE",
    "seh",
    "ses",
    "sg",
    "sgs",
    "shi",
    "shi-Latn",
    "shi-Tfng",
    "shn",
    "shn-TH",
    "si",
    "sid",
    "sk",
    "skr",
    "sl",
    "sma",
    "sma-NO",
    "smj",
    "smj-NO",
    "smn",
    "sms",
    "sn",
    "so",
    "so-DJ",
    "so-ET",
    "so-KE",
    "sq",
    "sq-MK",
    "sq-XK",
    "sr",
    "sr-Cyrl",
    "sr-Cyrl-BA",
    "sr-Cyrl-ME",
    "sr-Cyrl-XK",
    "sr-Latn",
    "sr-Latn-BA",
    "sr-Latn-ME",
    "sr-Latn-XK",
    "ss",
    "ss-SZ",
    "ssy",
    "st",
    "st-LS",
    "su",
    "su-Latn",
    "suz",
    "suz-Deva",
    "suz-Sunu",
    "sv",
    "sv-AX",
    "sv-FI",
    "sw",
    "sw-CD",
    "sw-KE",
    "sw-UG",
    "syr",
    "syr-SY",
    "szl",
    "ta",
    "ta-LK",
    "ta-MY",
    "ta-SG",
    "te",
    "teo",
    "teo-KE",
    "tg",
    "th",
    "ti",
    "ti-ER",
    "tig",
    "tk",
    "tn",
    "tn-BW",
    "to",
    "tok",
    "tpi",
    "tr",
    "tr-CY",
    "trv",
    "trw",
    "ts",
    "tt",
    "twq",
    "tyv",
    "tzm",
    "ug",
    "uk",
    "und",
    "ur",
    "ur-IN",
    "uz",
    "uz-Arab",
    "uz-Cyrl",
    "uz-Latn",
    "vai",
    "vai-Latn",
    "vai-Vaii",
    "ve",
    "vec",
    "vi",
    "vmw",
    "vo",
    "vun",
    "wa",
    "wae",
    "wal",
    "wbp",
    "wo",
    "xh",
    "xnr",
    "xog",
    "yav",
    "yi",
    "yo",
    "yo-BJ",
    "yrl",
    "yrl-CO",
    "yrl-VE",
    "yue",
    "yue-Hans",
    "yue-Hant",
    "yue-Hant-CN",
    "yue-Hant-MO",
    "za",
    "zgh",
    "zh",
    "zh-Hans",
    "zh-Hans-HK",
    "zh-Hans-MO",
    "zh-Hans-MY",
    "zh-Hans-SG",
    "zh-Hant",
    "zh-Hant-HK",
    "zh-Hant-MO",
    "zh-Hant-MY",
    "zh-Latn",
    "zu",
];

/// The available locales: [`CLDR_LOCALES`] and their default-content children (likely
/// subtags, `icu_locale::LocaleExpander`): for an id with no region, the region of its
/// maximized form (`en` -> `en-US`; with no script also the script, `zh` -> `zh-Hans-CN`,
/// `zh-CN`); for `lang-Script-RG`, `lang-RG` when the likely script of `lang-RG` is `Script`
/// (`zh-Hant-TW` -> `zh-TW`). Sorted, no repeats; made once per process.
pub fn locales() -> &'static [String] {
    static LOCALES: OnceLock<Vec<String>> = OnceLock::new();
    LOCALES.get_or_init(|| {
        let expander = LocaleExpander::new_extended();
        let mut out: Vec<String> = CLDR_LOCALES.iter().map(|s| (*s).to_string()).collect();
        let mut more: Vec<String> = Vec::new();
        // An id with no region: the region (and with no script, the script) of its maximized
        // form.
        for tag in CLDR_LOCALES {
            let Ok(id) = LanguageIdentifier::try_from_str(tag) else {
                continue;
            };
            if !id.variants.is_empty() || id.region.is_some() {
                continue;
            }
            let mut max = id.clone();
            expander.maximize(&mut max);
            let (Some(script), Some(region)) = (max.script, max.region) else {
                continue;
            };
            let mut child = id.clone();
            child.region = Some(region);
            more.push(child.to_string());
            if id.script.is_none() {
                child.script = Some(script);
                more.push(child.to_string());
                child.region = None;
                more.push(child.to_string());
            }
        }
        // `lang-Script-RG` gives `lang-RG` when the likely script of `lang-RG` is `Script`.
        let mut short_forms: Vec<String> = Vec::new();
        for tag in out.iter().chain(&more) {
            let Ok(id) = LanguageIdentifier::try_from_str(tag) else {
                continue;
            };
            let (Some(s), Some(_)) = (id.script, id.region) else {
                continue;
            };
            if !id.variants.is_empty() {
                continue;
            }
            let mut short = id.clone();
            short.script = None;
            let mut m = short.clone();
            expander.maximize(&mut m);
            if m.script == Some(s) {
                short_forms.push(short.to_string());
            }
        }
        more.extend(short_forms);
        out.extend(more);
        out.sort();
        out.dedup();
        out
    })
}

/// Whether `tag` (canonical, no Unicode extension) is an available locale.
pub fn is_available(tag: &str) -> bool {
    locales().binary_search_by(|l| l.as_str().cmp(tag)).is_ok()
}

/// AvailableCalendars (8.3.2 step 2; INTL-I1-D2): the 16 CLDR ids that temporal_rs 0.2.6 maps
/// to an icu_calendar `AnyCalendarKind`, sorted. It is `ep9_temporal`'s `AVAILABLE_CALENDARS`:
/// one list for Temporal and Intl (INTL-I1 §2.2, §4.4).
pub const CALENDARS: &[&str] = &ep9_temporal::calendar::AVAILABLE_CALENDARS;

/// AvailableCollations (8.3.2 step 3): the CLDR collation types (not "standard" or "search"),
/// sorted. W1 COLLATOR checks each against the ICU4X tailoring data
/// (`collations-accepted-by-Collator`).
pub const COLLATIONS: &[&str] = &[
    "compat", "dict", "emoji", "eor", "phonebk", "phonetic", "pinyin", "searchjl", "stroke",
    "trad", "unihan", "zhuyin",
];

/// AvailableCurrencies (8.3.2 step 4): the ISO 4217 codes in current use (the ICU list that
/// node v26.13.0 gives), sorted.
pub const CURRENCIES: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD",
    "CDF", "CHF", "CLP", "CNY", "COP", "CRC", "CUC", "CUP", "CVE", "CZK", "DJF", "DKK", "DOP",
    "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP", "GMD", "GNF",
    "GTQ", "GYD", "HKD", "HNL", "HRK", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR", "ISK",
    "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT", "LAK",
    "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU",
    "MUR", "MVR", "MWK", "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "OMR",
    "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB", "RWF", "SAR",
    "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SLL", "SOS", "SRD", "SSP", "STN", "SVC",
    "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH", "UGX",
    "USD", "UYU", "UZS", "VES", "VND", "VUV", "WST", "XAF", "XCD", "XCG", "XDR", "XOF", "XPF",
    "XSU", "YER", "ZAR", "ZMW", "ZWG", "ZWL",
];

/// AvailableNumberingSystems (8.3.2 step 5): the numbering systems with simple digit
/// mappings (ECMA-402 Table 14; the CLDR numberingSystems.xml "numeric" systems) that the
/// ICU4X decimal data has, sorted. A test formats digits with each one.
pub const NUMBERING_SYSTEMS: &[&str] = &[
    "adlm", "ahom", "arab", "arabext", "bali", "beng", "bhks", "brah", "cakm", "cham", "deva",
    "diak", "fullwide", "gara", "gong", "gonm", "gujr", "gukh", "guru", "hanidec", "hmng", "hmnp",
    "java", "kali", "kawi", "khmr", "knda", "krai", "lana", "lanatham", "laoo", "latn", "lepc",
    "limb", "mathbold", "mathdbl", "mathmono", "mathsanb", "mathsans", "mlym", "modi", "mong",
    "mroo", "mtei", "mymr", "mymrepka", "mymrpao", "mymrshan", "mymrtlng", "nagm", "newa", "nkoo",
    "olck", "onao", "orya", "osma", "outlined", "rohg", "saur", "segment", "shrd", "sind", "sinh",
    "sora", "sund", "sunu", "takr", "talu", "tamldec", "telu", "thai", "tibt", "tirh", "tnsa",
    "tols", "vaii", "wara", "wcho",
];

/// AvailableUnits (8.3.2 step 7): the sanctioned single units of ECMA-402 Table 2, sorted.
pub const UNITS: &[&str] = &[
    "acre",
    "bit",
    "byte",
    "celsius",
    "centimeter",
    "day",
    "degree",
    "fahrenheit",
    "fluid-ounce",
    "foot",
    "gallon",
    "gigabit",
    "gigabyte",
    "gram",
    "hectare",
    "hour",
    "inch",
    "kilobit",
    "kilobyte",
    "kilogram",
    "kilometer",
    "liter",
    "megabit",
    "megabyte",
    "meter",
    "microsecond",
    "mile",
    "mile-scandinavian",
    "milliliter",
    "millimeter",
    "millisecond",
    "minute",
    "month",
    "nanosecond",
    "ounce",
    "percent",
    "petabyte",
    "pound",
    "second",
    "stone",
    "terabit",
    "terabyte",
    "week",
    "yard",
    "year",
];

/// AvailablePrimaryTimeZoneIdentifiers (8.3.2 step 6): the primary IANA ids of the tz data
/// that Temporal uses (`ep9_temporal::tzdb::zone_ids`, INTL-I1-D9), sorted.
pub fn time_zones() -> &'static [&'static str] {
    ep9_temporal::tzdb::zone_ids()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted_unique(list: &[&str]) -> bool {
        list.windows(2).all(|w| w[0] < w[1])
    }

    #[test]
    fn lists_are_sorted_and_unique() {
        for (name, list) in [
            ("CLDR_LOCALES", CLDR_LOCALES),
            ("CALENDARS", CALENDARS),
            ("COLLATIONS", COLLATIONS),
            ("CURRENCIES", CURRENCIES),
            ("NUMBERING_SYSTEMS", NUMBERING_SYSTEMS),
            ("UNITS", UNITS),
        ] {
            assert!(sorted_unique(list), "{name}");
        }
        assert!(locales().windows(2).all(|w| w[0] < w[1]));
    }

    /// AvailableCalendars is one list for Temporal and Intl (INTL-I1 §2.2): `CALENDARS` is
    /// `ep9_temporal::calendar::AVAILABLE_CALENDARS`. This does not build while that module is
    /// private (the state at the INTL W1 merge head).
    #[test]
    fn calendars_are_the_temporal_list() {
        assert_eq!(CALENDARS, &ep9_temporal::calendar::AVAILABLE_CALENDARS[..]);
    }

    #[test]
    fn every_listed_locale_is_canonical() {
        for l in locales() {
            assert_eq!(
                crate::tag::canonicalize_tag(l).as_deref(),
                Some(l.as_str()),
                "{l}"
            );
        }
    }

    #[test]
    fn default_content_children_are_available() {
        let missing: Vec<&str> = [
            "en-US",
            "de-DE",
            "zh-Hans-CN",
            "zh-CN",
            "zh-Hant-TW",
            "zh-TW",
            "sr-BA",
            "en",
            "ja-JP",
            "pt-BR",
        ]
        .into_iter()
        .filter(|l| !is_available(l))
        .collect();
        assert!(missing.is_empty(), "{missing:?}");
        for l in ["en-ZZ", "xx", "zh-Hant-CN"] {
            assert!(!is_available(l), "{l}");
        }
    }

    /// Each numbering system has a simple digit mapping (`digits::simple_digit`), and where
    /// the ICU4X decimal data has the system, its digits are the same.
    #[test]
    fn every_numbering_system_has_digits() {
        use fixed_decimal::Decimal;
        use icu_decimal::DecimalFormatter;
        use icu_locale_core::Locale;
        let digits = |nu: &str| {
            let loc: Locale = format!("und-u-nu-{nu}").parse().unwrap();
            let f = DecimalFormatter::try_new(loc.into(), Default::default()).unwrap();
            f.format(&Decimal::from(1_234_567_890u64)).to_string()
        };
        let latn = digits("latn");
        let mut with_data = 0;
        for nu in NUMBERING_SYSTEMS {
            let want: String = [1, 2, 3, 4, 5, 6, 7, 8, 9, 0]
                .iter()
                .map(|d| crate::digits::simple_digit(nu, *d).unwrap())
                .collect();
            let got: String = digits(nu).chars().filter(|c| *c != ',').collect();
            if *nu == "latn" || got != latn.replace(',', "") {
                assert_eq!(got, want, "{nu}");
                with_data += 1;
            }
        }
        // The ICU4X 2.3 data has 28 of the 78 systems (latn included).
        assert!(with_data >= 20, "{with_data}");
    }

    /// The two tz sources can differ in version (INTL-I1 §4.4): every primary zone id of the
    /// Temporal tz data (read through `ep9_temporal::tzdb::zone_ids`, no zoneinfo64 edge)
    /// parses in the ICU4X `icu_time` IANA parser, except the ids in `NEWER` (zones of tzdb
    /// 2026f that the ICU4X 2.3 data does not have yet; DateTimeFormat (W4) formats their
    /// names with the offset form). A new id in either source fails this test.
    #[test]
    fn every_zone_id_parses_in_icu_time() {
        use icu_time::TimeZone;
        use icu_time::zone::IanaParser;
        let parser = IanaParser::new();
        let unknown: Vec<&str> = time_zones()
            .iter()
            .copied()
            .filter(|id| parser.parse(id) == TimeZone::UNKNOWN)
            .collect();
        const NEWER: &[&str] = &["Asia/Hanoi"];
        assert_eq!(unknown, NEWER);
        assert!(time_zones().len() > 400, "{}", time_zones().len());
    }
}

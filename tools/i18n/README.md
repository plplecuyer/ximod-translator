# Interface locales (E-07)

`source.json` : the keys of XIMOD Translator that XIMOD Architect does not
share (English and French texts). `shared_keys.json` : the keys whose
English text is identical in Architect, taken from its 32 locales.
`<code>.json` : the translations of `source.json` for one language,
produced by machine translation and flagged in the locale header.
`build_locales.py` assembles `assets/locales/<code>/main.ftl` from the
English reference structure.

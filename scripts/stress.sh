#!/usr/bin/env bash
# Stress test for mdqy. ~110 cases, hunting for parse, eval,
# mutation, multi-file, encoder, and jq-compat bugs.
#
# Usage: scripts/stress.sh         (build release if missing, run)
#        scripts/stress.sh --debug (use dev profile)
#        scripts/stress.sh -v      (print every assertion)
#
# Exits 0 if every test passes, 1 if any fails.
#
# Known divergences this script asserts as failures (so regressions
# resurface immediately):
#   - `not` lexes as Tok::KwNot and only parses as a unary prefix,
#     so `5 | not` is a parse error (jq accepts both forms).
#   - `try EXPR` form unsupported; only `EXPR?` works.
#   - `length` on a number errors out; jq returns abs(n).
#   - `ascii_upcase` / `ascii_downcase` only handle ASCII
#     (intentional per builtins.rs, divergent from jq).

set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE=release
VERBOSE=0
for arg in "$@"; do
    case "$arg" in
        --debug) PROFILE=debug ;;
        -v|--verbose) VERBOSE=1 ;;
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    esac
done

if [[ "$PROFILE" == "release" ]]; then
    MDQY="$ROOT/target/release/mdqy"
    BUILD="--release"
else
    MDQY="$ROOT/target/debug/mdqy"
    BUILD=""
fi

if [[ ! -x "$MDQY" ]] || [[ "$ROOT/Cargo.toml" -nt "$MDQY" ]]; then
    echo "building $PROFILE binary..." >&2
    ( cd "$ROOT" && cargo build $BUILD --features tty,watch ) >/dev/null 2>&1 || {
        echo "build failed"; exit 1
    }
fi

TMP=$(mktemp -d /tmp/mdqy-stress.XXXXXX)
trap "rm -rf '$TMP'" EXIT
cd "$TMP" || exit 1

PASS=0
FAIL=0
declare -a FAILED

if [[ -t 1 ]]; then
    R=$'\033[31m'; G=$'\033[32m'; Y=$'\033[33m'; D=$'\033[90m'; X=$'\033[0m'
else
    R=''; G=''; Y=''; D=''; X=''
fi

ok() {
    PASS=$((PASS + 1))
    if (( VERBOSE )); then printf '%sok%s   %s\n' "$G" "$X" "$1"
    else printf '%s.%s' "$G" "$X"; fi
}

ko() {
    FAIL=$((FAIL + 1))
    FAILED+=("$1")
    if (( VERBOSE )); then printf '%sFAIL%s %s\n' "$R" "$X" "$1"
    else printf '%s!%s' "$R" "$X"; fi
}

# `$()` strips trailing newlines. To preserve exact bytes we append a
# sentinel `x` past the binary's output and strip it back. Every
# helper that compares output captures the full byte stream this way
# so `ts_eq` can match a trailing `\n`.

# Substring / equality assertions. `seq_eq` strips a single trailing
# newline from each side so that "5" matches mdqy's "5\n" output;
# identity tests still match because both source and serialised output
# either both end in `\n` or neither does.
sin() { [[ "$3" == *"$2"* ]] && ok "$1" || ko "$1 :: want substr='$2' got='${3:0:200}'"; }
nin() { [[ "$3" != *"$2"* ]] && ok "$1" || ko "$1 :: forbidden substr='$2' present in '${3:0:200}'"; }
seq_eq() {
    local w="${2%$'\n'}" g="${3%$'\n'}"
    [[ "$g" == "$w" ]] && ok "$1" || ko "$1 :: want='${2:0:200}' got='${3:0:200}'"
}

# Stdin-fed tests --------------------------------------------------------
ts_in() {
    local n="$1" w="$2" s="$3" e="$4"; shift 4
    local got=$({ printf '%s' "$s" | "$MDQY" --stdin "$@" "$e" 2>&1; printf x; })
    sin "$n" "$w" "${got%x}"
}
ts_nin() {
    local n="$1" w="$2" s="$3" e="$4"; shift 4
    local got=$({ printf '%s' "$s" | "$MDQY" --stdin "$@" "$e" 2>&1; printf x; })
    nin "$n" "$w" "${got%x}"
}
ts_eq() {
    local n="$1" w="$2" s="$3" e="$4"; shift 4
    local got=$({ printf '%s' "$s" | "$MDQY" --stdin "$@" "$e" 2>&1; printf x; })
    seq_eq "$n" "$w" "${got%x}"
}
ts_fail() {
    local n="$1" s="$2" e="$3"; shift 3
    if printf '%s' "$s" | "$MDQY" --stdin "$@" "$e" >/dev/null 2>&1; then
        ko "$n :: expected failure, got success"
    else ok "$n"; fi
}

# Null-input tests -------------------------------------------------------
tn_in() {
    local n="$1" w="$2" e="$3"; shift 3
    local got=$({ "$MDQY" -n "$@" "$e" 2>&1; printf x; })
    sin "$n" "$w" "${got%x}"
}
tn_eq() {
    local n="$1" w="$2" e="$3"; shift 3
    local got=$({ "$MDQY" -n "$@" "$e" 2>&1; printf x; })
    seq_eq "$n" "$w" "${got%x}"
}
tn_fail() {
    local n="$1" e="$2"; shift 2
    if "$MDQY" -n "$@" "$e" >/dev/null 2>&1; then
        ko "$n :: expected failure, got success"
    else ok "$n"; fi
}

section() {
    if (( VERBOSE )); then printf '\n%s---- %s ----%s\n' "$Y" "$1" "$X"
    else printf '\n%s%-25s%s ' "$D" "$1" "$X"; fi
}

# ---- fixtures --------------------------------------------------------------

TINY=$'# Tiny\n\nA paragraph with [a link](http://example.com).\n\n## Second heading\n\n```rust\nfn main() {}\n```\n'
DEEP=$'# A\n\n## A.1\n\n### A.1.1\n\nleaf one.\n\n### A.1.2\n\nleaf two.\n\n## A.2\n\nbody.\n\n# B\n\n## B.1\n\nlast.\n'
WITH_FM=$'---\ntitle: Hello\ntags:\n  - a\n  - b\nnumber: 42\n---\n\n# Body\n\nText.\n'
WITH_TOML=$'+++\ntitle = "Hello"\ncount = 7\n+++\n\n# Body\n\nText.\n'
UNICODE=$'# Café\n\nGreek: αβγ. Han: 中文. Emoji: 🍕🚀.\n'
LIST_DOC=$'# Lists\n\n- one\n- two\n  - nested\n- three\n'
CODE_DOC=$'# Code\n\n```python\nprint("hi")\n```\n\n```rust\nfn main() {}\n```\n\n```\nno-lang\n```\n'
LINKS_DOC=$'# Links\n\n[plain](http://a.com)\n\n[titled](http://b.com "Title here")\n\n![alt-text](http://c.com/img.png "img-title")\n'
TABLE_DOC=$'# T\n\n| H1 | H2 |\n| --- | --- |\n| a | b |\n| c | d |\n'

# ============================================================================
section "A. identity"

ts_eq A_id_tiny     "$TINY"    "$TINY"    '.'
ts_eq A_id_deep     "$DEEP"    "$DEEP"    '.'
ts_eq A_id_unicode  "$UNICODE" "$UNICODE" '.'
ts_eq A_id_list     "$LIST_DOC" "$LIST_DOC" '.'
ts_eq A_id_table    "$TABLE_DOC" "$TABLE_DOC" '.'
ts_eq A_id_empty    ""         ""         '.'

# Bug: walk(.) should be a byte-exact roundtrip on a clean tree.
ts_eq A_walk_identity_tiny "$TINY" "$TINY" 'walk(.)' --output md
ts_eq A_walk_identity_deep "$DEEP" "$DEEP" 'walk(.)' --output md

# ============================================================================
section "B. selectors"

ts_in  B_h_text         "Tiny"             "$TINY" 'headings | .text'
ts_in  B_h_text2        "Second heading"   "$TINY" 'headings | .text'
ts_in  B_h1_text        "Tiny"             "$TINY" 'h1 | .text'
ts_in  B_h2_text        "Second heading"   "$TINY" 'h2 | .text'
ts_nin B_h2_no_h1       "Tiny"             "$TINY" 'h2 | .text'
ts_in  B_code_lang      "rust"             "$TINY" 'codeblocks | .lang'
ts_in  B_code_literal   "fn main"          "$TINY" 'codeblocks | .literal'
ts_in  B_links_href     "http://example"   "$TINY" 'links | .href'
ts_in  B_anchor_h1      "tiny"             "$TINY" 'h1 | .anchor'
ts_in  B_anchor_h2      "second-heading"   "$TINY" 'h2 | .anchor'
ts_in  B_paragraphs     "paragraph with"   "$TINY" 'paragraphs | .text'
ts_fail B_h7_unknown    "$TINY" 'h7 | .text'
ts_in  B_lang_filter    "rust"             "$CODE_DOC" 'codeblocks:lang(rust) | .lang'
ts_nin B_lang_no_python "python"           "$CODE_DOC" 'codeblocks:lang(rust) | .lang'
ts_in  B_images_href    "img.png"          "$LINKS_DOC" 'images | .href'

# Image alt text not lifted to attr (bug). The `alt` attr is the
# bracket text in markdown image syntax. Test is intentionally strict.
ts_in  B_images_alt     "alt-text"         "$LINKS_DOC" 'images | .alt'

# ============================================================================
section "C. pseudos"

ts_in  C_first      "Tiny"            "$TINY" 'headings:first | .text'
ts_in  C_last       "Second heading"  "$TINY" 'headings:last | .text'
ts_in  C_nth0       "Tiny"            "$TINY" 'headings:nth(0) | .text'
ts_in  C_nth1       "Second heading"  "$TINY" 'headings:nth(1) | .text'
ts_in  C_nth_neg1   "Second heading"  "$TINY" 'headings:nth(-1) | .text'
ts_eq  C_nth_far    "null"            "$TINY" 'headings:nth(99) | .text'
ts_eq  C_nth_neg_far "null"           "$TINY" 'headings:nth(-99) | .text'
ts_in  C_text_quote "Tiny"            "$TINY" 'headings:text("Tiny") | .text'

# ============================================================================
section "D. sections / combinator"

ts_in  D_section_ascii "Second heading" "$TINY" 'section("Second heading") | .text'
ts_in  D_section_case  "Second heading" "$TINY" 'section("SECOND HEADING") | .text'
ts_eq  D_section_miss  ""               "$TINY" 'section("Nope") | .text' --raw
ts_in  D_hash_title    "Tiny"           "$TINY" '# Tiny | .text'
ts_in  D_hash_quoted   "Second heading" "$TINY" '## "Second heading" | .text'
ts_in  D_combinator    "fn main"        "$TINY" '# "Second heading" > codeblocks | .literal'
ts_in  D_combinator2   "leaf one"       "$DEEP" '# A > # "A.1" > # "A.1.1" | .text'

# ============================================================================
section "E. field / index / slice"

ts_eq E_kind_root    "root"        "$TINY" '.kind' --raw
ts_in E_text_full    "Tiny"        "$TINY" '.text'
ts_in E_text_para    "paragraph"   "$TINY" '.text'
# .[-1] is the last block-level child. The TINY doc emits
# (heading h1, paragraph, heading h2, code). Last is code.
ts_eq E_index_neg_kind "\"code\"" "$TINY" '.[-1] | .kind' -c
ts_eq E_index_far    "null"        "$TINY" '.[99]'

# Slice
ts_in  E_slice_kinds "heading"      "$TINY" '.[0:1] | .[] | .kind'
ts_eq  E_slice_inverted "[]"        "$TINY" '.[5:1]' -c

# Field on null is null
tn_eq E_field_null   "null"  '.foo'
# .. recurse all
ts_in E_recurse      "Second heading" "$TINY" '.. | select(.kind == "heading") | .text'

# ============================================================================
section "F. predicates"

tn_eq F_select_pass    "5"     '5 | select(. > 3)'
tn_eq F_select_drop    ""      '5 | select(. > 99)'
tn_eq F_any_empty      "false" '[] | any'
tn_eq F_all_empty      "true"  '[] | all'
tn_eq F_any_one_true   "true"  '[false, true] | any'
tn_eq F_all_one_false  "false" '[true, false] | all'

# `not` builtin: jq accepts `5 | not`. Mdqy lexes `not` as keyword
# (Tok::KwNot) so it can only parse as a unary prefix. Pipe form
# parse-errors. Surface the divergence.
tn_fail F_not_pipe_form_BUG  '5 | not'
tn_eq   F_not_prefix_null   "true"  'not null'
tn_eq   F_not_prefix_zero   "false" 'not 0'
tn_eq   F_not_prefix_false  "true"  'not false'

# ============================================================================
section "G. numbers / range"

tn_eq G_arith         "5"        '2 + 3'
tn_eq G_div           "2"        '6 / 3'
tn_eq G_mod           "1"        '7 % 2'
tn_eq G_neg           "-3"       '0 - 3'
tn_eq G_int_emit      "1"        '1.0'
tn_eq G_float_emit    "1.5"      '1.5'
tn_eq G_huge          "1e+300"   '1e300'  # JSON formatter normalises exponent

tn_eq G_range_pos     "[0,1,2,3,4]" '[range(5)]' -c
tn_eq G_range_neg     "[]"          '[range(-3)]' -c
tn_eq G_range_step    "[5,4,3,2,1]" '[range(5; 0; -1)]' -c
tn_fail G_range_zero_step           'range(0; 5; 0)'

tn_eq G_limit_2       "[0,1]"       '[limit(2; range(10))]' -c
tn_eq G_limit_0       "[]"          '[limit(0; range(10))]' -c
tn_eq G_limit_neg     "[]"          '[limit(-5; range(10))]' -c
tn_eq G_nth_2         "2"           'nth(2; range(10))'
tn_eq G_nth_far       "null"        'nth(100; range(10))'
tn_eq G_nth_neg       "null"        'nth(-1; range(10))'

# ============================================================================
section "H. strings / regex"

tn_eq H_concat        '"hello world"'  '"hello" + " " + "world"'
tn_eq H_split         '["a","b","c"]'  '"a,b,c" | split(",")' -c
tn_eq H_join          '"a-b-c"'        '["a","b","c"] | join("-")'
tn_eq H_upcase        '"FOO"'          '"foo" | ascii_upcase'
tn_eq H_downcase      '"bar"'          '"BAR" | ascii_downcase'
# ascii_upcase only touches ASCII. Documents the behaviour.
tn_eq H_upcase_partial '"CAFé"'        '"café" | ascii_upcase'

tn_eq H_test_match    "true"           '"hello" | test("h.+o")'
tn_eq H_test_no       "false"          '"hello" | test("xyz")'
tn_fail H_test_bad                     '"x" | test("[unclosed")'

tn_eq H_sub           '"hxllo hello"'  '"hello hello" | sub("e"; "x")'
tn_eq H_gsub          '"hxllo hxllo"'  '"hello hello" | gsub("e"; "x")'

tn_eq H_starts        "true"           '"hello" | startswith("hel")'
tn_eq H_ends          "true"           '"hello" | endswith("llo")'
tn_eq H_contains      "true"           '"hello world" | contains("world")'
tn_eq H_ltrimstr      '"world"'        '"prefix-world" | ltrimstr("prefix-")'
tn_eq H_rtrimstr      '"world"'        '"world-suffix" | rtrimstr("-suffix")'
tn_eq H_ltrimstr_id   '"world"'        '"world" | ltrimstr("xyz")'

# Length
tn_eq H_len_str       "5"              '"hello" | length'
tn_eq H_len_arr       "3"              '[1,2,3] | length'
tn_eq H_len_obj       "2"              '{a:1,b:2} | length'
tn_eq H_len_null      "0"              'null | length'
# jq divergence: length(n) = abs(n). Mdqy errors.
tn_fail H_len_num_BUG                  '5 | length'

# Interpolation
tn_eq H_interp_arith  '"x=5"'          '"x=\(2+3)"'
tn_eq H_interp_field  '"name=Alice"'   '{name:"Alice"} | "name=\(.name)"'
tn_eq H_interp_null   '"x=null"'       '{} | "x=\(.foo)"'

# ============================================================================
section "I. encoders"

tn_eq I_csv_simple    '"1,\"a, b\",true"'  '[1, "a, b", true] | @csv'
tn_eq I_csv_quote     '"\"\"\"hi\"\"\""'   '["\"hi\""] | @csv'
tn_eq I_tsv           '"a\tb\tc"'          '["a","b","c"] | @tsv'
tn_eq I_uri_space     '"hello%20world"'    '"hello world" | @uri'
tn_eq I_uri_unicode   '"caf%C3%A9"'        '"café" | @uri'
tn_eq I_html_lt       '"&lt;tag&gt;"'      '"<tag>" | @html'
tn_eq I_html_amp      '"a&amp;b"'          '"a&b" | @html'
tn_eq I_html_quote    '"&#39;hi&#39;"'     $'"\047hi\047" | @html'
tn_in I_sh_works      "hello"              '"hello" | @sh'
tn_in I_sh_apostrophe "\\'"                '"don'\''t" | @sh'
tn_eq I_json_compact  '"[1,2,3]"'          '[1,2,3] | @json'

# ============================================================================
section "J. object / array"

tn_eq J_obj           '{"a":1,"b":2}'      '{a:1, b:2}' -c
tn_eq J_obj_short     '{"x":5}'            '{x:5} | {x}' -c
tn_eq J_obj_compkey   '{"NAME":"v"}'       '{("NAME"): "v"}' -c
tn_eq J_arr           '[1,2,3]'            '[1,2,3]' -c
tn_eq J_keys_obj      '["a","b"]'          '{b:2,a:1} | keys' -c
tn_eq J_keys_arr      '[0,1,2]'            '[10,20,30] | keys' -c
tn_eq J_has_obj_yes   "true"               '{x:1} | has("x")'
tn_eq J_has_obj_no    "false"              '{x:1} | has("y")'
tn_eq J_has_arr_yes   "true"               '[1,2,3] | has(1)'
tn_eq J_has_arr_no    "false"              '[1,2,3] | has(99)'
tn_eq J_add_nums      "6"                  '[1,2,3] | add'
tn_eq J_add_strs      '"abc"'              '["a","b","c"] | add'
tn_eq J_add_arrs      '[1,2,3,4]'          '[[1,2],[3,4]] | add' -c
tn_eq J_add_empty     "null"               '[] | add'
tn_eq J_sort          '[1,2,3]'            '[3,1,2] | sort' -c
tn_eq J_sort_mixed    '[null,false,1,"a"]' '[1, "a", null, false] | sort' -c
tn_eq J_sort_by_len   '["a","bb","ccc"]'   '["bb","a","ccc"] | sort_by(length)' -c
tn_eq J_unique        '[1,2,3]'            '[3,1,2,1,3] | unique' -c
tn_eq J_group_by      '[[2],[1,3]]'        '[1,2,3] | group_by(. % 2)' -c
tn_eq J_min_by        '"a"'                '["bb","a","ccc"] | min_by(length)'
tn_eq J_max_by        '"ccc"'              '["bb","a","ccc"] | max_by(length)'
tn_eq J_min           "1"                  '[3,1,2] | min'
tn_eq J_max           "3"                  '[3,1,2] | max'
tn_eq J_min_empty     "null"               '[] | min'
tn_eq J_slice_neg     '[2,3]'              '[1,2,3] | .[-2:]' -c
tn_eq J_slice_pos     '[1]'                '[1,2,3] | .[:1]' -c
tn_eq J_slice_oob     '[1,2,3]'            '[1,2,3] | .[0:99]' -c
tn_eq J_slice_inv     '[]'                 '[1,2,3] | .[2:1]' -c

# ============================================================================
section "K. paths"

tn_eq K_paths_obj     '[["a"]]'                  '{a:1} | [paths]' -c
tn_eq K_paths_nested  '[["a"],["a","b"]]'        '{a:{b:1}} | [paths]' -c
tn_eq K_getpath       "1"                        '{a:{b:1}} | getpath(["a","b"])'
tn_eq K_getpath_miss  "null"                     '{a:1} | getpath(["x","y"])'
tn_eq K_getpath_empty '{"a":1}'                  '{a:1} | getpath([])' -c
tn_eq K_setpath_new   '{"a":{"b":5}}'            'null | setpath(["a","b"]; 5)' -c
tn_eq K_setpath_over  '{"a":{"b":2}}'            '{a:{b:1}} | setpath(["a","b"]; 2)' -c
tn_eq K_setpath_arr   '[null,null,"x"]'          '[] | setpath([2]; "x")' -c
tn_eq K_setpath_empty "5"                        '{} | setpath([]; 5)'

# ============================================================================
section "L. type / conv"

tn_eq L_type_null  '"null"'    'null | type'
tn_eq L_type_str   '"string"'  '"x" | type'
tn_eq L_type_num   '"number"'  '5 | type'
tn_eq L_type_bool  '"boolean"' 'true | type'
tn_eq L_type_arr   '"array"'   '[] | type'
tn_eq L_type_obj   '"object"'  '{} | type'
tn_eq L_tos_null   '"null"'    'null | tostring'
tn_eq L_tos_int    '"5"'       '5 | tostring'
tn_eq L_tos_float  '"1.5"'     '1.5 | tostring'
tn_eq L_tos_bool   '"true"'    'true | tostring'
tn_eq L_tos_arr    '"[1,2,3]"' '[1,2,3] | tostring'
tn_eq L_tos_obj    '"{\"a\":1}"' '{a:1} | tostring'
tn_eq L_tos_str    '"keep"'    '"keep" | tostring'
tn_eq L_ton_str    "42"        '"42" | tonumber'
tn_eq L_ton_float  "3.14"      '"3.14" | tonumber'
tn_fail L_ton_bad             '"abc" | tonumber'
tn_fail L_ton_bool            'true | tonumber'
tn_eq L_toj_obj    '"{\"a\":1}"'  '{a:1} | tojson'
tn_eq L_fromj      '{"a":1}'    '"{\"a\":1}" | fromjson' -c
tn_fail L_fromj_bad            '"{not json" | fromjson'

# ============================================================================
section "M. mutation"

DOC_HTTP=$'See [docs](http://example.com).\n'
DOC_HTTPS=$'See [docs](https://example.com).\n'
ts_eq M_link_rewrite "$DOC_HTTPS" "$DOC_HTTP" \
    '(.. | select(type == "link")).href |= sub("http:"; "https:")' --output md

ts_eq M_link_idempotent "$DOC_HTTPS" "$DOC_HTTPS" \
    '(.. | select(type == "link")).href |= sub("http:"; "https:")' --output md

DOC_TITLED=$'See [docs](https://example.com "title").\n'
ts_nin M_del_title "title" "$DOC_TITLED" \
    'del((.. | select(type == "link")).title)' --output md

# Mutation with no targets: identity output.
ts_eq M_no_targets "$TINY" "$TINY" \
    '(.. | select(type == "image")).href |= sub("http:"; "https:")' --output md

# Walk that bumps levels.
ts_in M_walk_bump "## Tiny" "$TINY" \
    'walk(if .kind == "heading" then .level |= . + 1 else . end)' --output md

# walk(.) keeps text content (even if not byte-exact).
ts_in M_walk_text "Tiny" "$TINY" 'walk(.)' --output md

# del then output: title gone from emitted markdown.
ts_nin M_del_title_in_output "title" "$DOC_TITLED" \
    'del((.. | select(type == "link")).title)' --output md

# `=` (set) still NotImplemented; should fail when run via -U.
TFI=$(mktemp "$TMP/setfail.XXXXXX.md"); printf '%s' "$DOC_HTTP" > "$TFI"
"$MDQY" -U '.text = "x"' "$TFI" >/dev/null 2>&1 \
    && ko "M_set_assign_fails :: -U with `=` should error" \
    || ok M_set_assign_fails

# Unknown attr name on a mutation target: fail.
TFA=$(mktemp "$TMP/badattr.XXXXXX.md"); printf '%s' "$TINY" > "$TFA"
"$MDQY" -U 'h1.bogus_attr |= "x"' "$TFA" >/dev/null 2>&1 \
    && ko "M_unknown_attr_fails :: bogus attr should error" \
    || ok M_unknown_attr_fails

# Mutation on --stdin should not silently no-op (potential bug).
got_mut=$(printf '%s' "$DOC_HTTP" | "$MDQY" --stdin --output md \
    '(.. | select(type == "link")).href |= sub("http:"; "https:")' 2>&1)
if [[ "$got_mut" == *"https://"* ]]; then ok M_stdin_mutation_runs
else ko "M_stdin_mutation_runs :: stdin+mutation produced '${got_mut:0:80}'"
fi

# ============================================================================
section "N. multi-file & flags"

mkdir -p docs
printf '# A\n\nA body.\n' > docs/a.md
printf '# B\n\nB body.\n' > docs/b.md
printf '# Hidden\n' > docs/.hidden.md
echo 'docs/.hidden.md' > .ignore

got=$("$MDQY" 'headings | .text' docs/ 2>&1)
[[ "$got" == *"A"* && "$got" == *"B"* ]] && ok N_per_file || ko "N_per_file :: '$got'"

got=$("$MDQY" --slurp 'length' docs/ 2>&1)
[[ "$got" == "2" ]] && ok N_slurp || ko "N_slurp :: '$got'"

got=$("$MDQY" --merge 'headings | .text' docs/ 2>&1)
[[ "$got" == *"A"* && "$got" == *"B"* ]] && ok N_merge || ko "N_merge :: '$got'"

got=$("$MDQY" --no-ignore --hidden --slurp 'length' docs/ 2>&1)
[[ "$got" == "3" ]] && ok N_no_ignore_hidden || ko "N_no_ignore_hidden :: '$got'"

got_s=$("$MDQY" --workers 1 'headings | .text' docs/ 2>&1)
got_p=$("$MDQY" --workers 4 'headings | .text' docs/ 2>&1)
[[ "$got_s" == "$got_p" ]] && ok N_workers_match || ko "N_workers_match :: serial='$got_s' parallel='$got_p'"

got=$("$MDQY" -n '1+2'); [[ "$got" == "3" ]] && ok N_null_input || ko "N_null_input :: '$got'"

got=$(printf '%s' "$TINY" | "$MDQY" --stdin --raw '.text' 2>&1)
[[ "$got" != *'"'* ]] && ok N_raw_strips_quotes || ko "N_raw_strips_quotes :: '$got'"

got=$(printf 'hello world' | "$MDQY" -R --stdin '.')
[[ "$got" == *"hello world"* ]] && ok N_raw_input || ko "N_raw_input :: '$got'"

got=$("$MDQY" -n --arg name "Bob" '"hi \($name)"')
[[ "$got" == *"hi Bob"* ]] && ok N_arg || ko "N_arg :: '$got'"

got=$("$MDQY" -n --argjson n 7 '$n + 3')
[[ "$got" == "10" ]] && ok N_argjson || ko "N_argjson :: '$got'"

"$MDQY" -n --argjson n 'not-json' '$n' >/dev/null 2>&1 \
    && ko "N_argjson_bad :: should fail" || ok N_argjson_bad

"$MDQY" --compile-only 'not | valid |' >/dev/null 2>&1 \
    && ko "N_compile_bad :: should fail" || ok N_compile_bad

got=$("$MDQY" --explain-mode 'headings | .text')
[[ "$got" == "mode: stream" ]] && ok N_explain_stream || ko "N_explain_stream :: '$got'"
got=$("$MDQY" --explain-mode '[headings] | length')
[[ "$got" == "mode: tree" ]] && ok N_explain_tree || ko "N_explain_tree :: '$got'"

"$MDQY" --slurp --merge '.' docs/ >/dev/null 2>&1 \
    && ko "N_slurp_merge_conflict :: should fail" || ok N_slurp_merge_conflict

got=$(printf '%s' "$TINY" | "$MDQY" --output text --stdin '.')
[[ "$got" == *"Tiny"* && "$got" != *"#"* ]] && ok N_output_text \
    || ko "N_output_text :: '${got:0:80}'"

TF=$(mktemp "$TMP/inplace.XXXXXX.md")
printf 'See [x](http://e.com).\n' > "$TF"
"$MDQY" -U '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TF" 2>&1 >/dev/null
[[ "$(cat "$TF")" == *"https://"* ]] && ok N_in_place || ko "N_in_place :: '$(cat "$TF")'"

TB=$(mktemp "$TMP/backup.XXXXXX.md")
printf 'See [x](http://e.com).\n' > "$TB"
"$MDQY" -U --backup bak '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TB" 2>&1 >/dev/null
if compgen -G "${TB}.bak" >/dev/null || compgen -G "${TB%.md}.md.bak" >/dev/null; then
    ok N_backup_made
else
    ko "N_backup_made :: no backup near $TB ($(ls -1 "$TMP" | grep -i backup))"
fi

# ============================================================================
section "O. compile errors"

ts_fail O_paren        ''         '(. '
ts_fail O_brack        ''         '[. '
ts_fail O_brace        ''         '{ a: '
ts_fail O_trailing     ''         '. |'
ts_fail O_double_pipe  ''         '. || .'
ts_fail O_unterm_str   ''         '"oops'
ts_fail O_pseudo       "$TINY"    'headings:bogus | .text'
ts_fail O_unknown_fn   "$TINY"    'thiss_does_not_exist'

# `try EXPR` form not supported (only `EXPR?`). Mark as bug.
tn_fail O_try_form_BUG '' 'try error("bang")'

# But `?` postfix works.
tn_eq   O_try_postfix  ""  'error("bang")?'

# ============================================================================
section "P. frontmatter"

ts_in p_yaml_title  "Hello"  "$WITH_FM"   'frontmatter | .title'
ts_in p_yaml_tag    "a"      "$WITH_FM"   'frontmatter | .tags | .[0]' --raw
ts_in p_yaml_num    "42"     "$WITH_FM"   'frontmatter | .number'
ts_in p_toml_title  "Hello"  "$WITH_TOML" 'frontmatter | .title'
ts_in p_toml_count  "7"      "$WITH_TOML" 'frontmatter | .count'
ts_eq p_fm_missing  "null"   "$TINY"      'frontmatter'

# ============================================================================
section "Q. stream/tree parity"

# For each stream-eligible expression, run it through the binary
# (which dispatches to stream mode) and through the binary in tree
# mode (forced by wrapping in a noop that has_mutation rejects).
# Outputs must match exactly.
parity() {
    local name="$1" doc="$2" expr="$3"
    local mode_a out_a out_b
    mode_a=$("$MDQY" --explain-mode "$expr")
    out_a=$(printf '%s' "$doc" | "$MDQY" --stdin "$expr")
    # Force tree by appending `| .` (still stream-eligible per plan)
    # so we instead force tree by wrapping in a comma `., .` then
    # taking first; stream plan rejects comma.
    out_b=$(printf '%s' "$doc" | "$MDQY" --stdin "$expr,$expr | first")
    # The above isn't a great forcing — instead compare to `[expr] | .[]`
    # which has [array] which kicks tree.
    out_b=$(printf '%s' "$doc" | "$MDQY" --stdin "[$expr] | .[]")
    if [[ "$out_a" == "$out_b" ]]; then ok "$name"
    else ko "$name :: stream='${out_a:0:80}' tree='${out_b:0:80}'"
    fi
}

parity Q_h_text       "$DEEP" 'headings | .text'
parity Q_h_anchor     "$DEEP" 'headings | .anchor'
parity Q_h_lvl_filter "$DEEP" 'headings | select(.level == 2) | .text'
parity Q_code_lang    "$CODE_DOC" 'codeblocks | .lang'
parity Q_code_lit     "$CODE_DOC" 'codeblocks | .literal'
parity Q_links_href   "$LINKS_DOC" 'links | .href'

# ============================================================================
section "R. JSON schema"

ts_in  R_json_kind     '"kind":"heading"' "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_in  R_json_text     '"text":"Tiny"'    "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_in  R_json_int_lvl  '"level":1'        "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_nin R_json_no_float '1.0'              "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_nin R_json_no_span  '"span":'          "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_in  R_json_with_span '"span":'         "$TINY" '.. | select(.kind == "heading")' --output json --with-spans -c
ts_nin R_json_no_empty_children '"children":[]' "$TINY" 'h1' --output json -c

# ============================================================================
section "S. edges"

ts_eq S_empty_id        ""    "" '.'
ts_eq S_empty_kind      "root" "" '.kind' --raw
ts_eq S_empty_text      ""    "" '.text' --raw

DOC_FM_ONLY=$'---\nx: 1\n---\n'
ts_in S_fm_only_x  "1" "$DOC_FM_ONLY" 'frontmatter | .x'

DOC_RICH=$'# Hello **bold** `code`\n\nbody.\n'
ts_in S_rich       "Hello bold code" "$DOC_RICH" 'h1 | .text'

DOC_SETEXT=$'Setext\n======\n\npara\n'
ts_in S_setext     "Setext"  "$DOC_SETEXT" 'h1 | .text'

# Try / error
tn_eq S_try_postfix  "" 'error("bang")?'
tn_fail S_no_try_fails  'error("bang")'

# Reduce / foreach
tn_eq S_reduce_sum   "10"            '[1,2,3,4] | reduce .[] as $x (0; . + $x)'
tn_eq S_foreach      "[1,3,6,10]"    '[1,2,3,4] | [foreach .[] as $x (0; . + $x; .)]' -c

# User defs
tn_eq S_def          "12"            'def double(x): x + x; double(6)'
tn_eq S_def_two      "12"            'def add2(x; y): x + y; add2(5; 7)'

# As binding
tn_eq S_as           "10"            '5 as $x | $x + $x'

# Undefined variable
tn_fail S_undef                      '$undefined'

# Recurse on nested
# `..` walks every value including the top.
tn_eq S_recurse_full "[[[1,2]],[1,2],1,2]" '[[1,2]] | [..]' -c

# Comma operator
tn_eq S_comma        '[1,2,3]'       '[1,2,3]' -c

# Trailing newline preserved through identity
DOC_NL=$'# x\n'
ts_eq S_trailing_nl  "$DOC_NL" "$DOC_NL" '.'

# CRLF input survives identity
DOC_CRLF=$'# x\r\n\r\nbody\r\n'
ts_eq S_crlf         "$DOC_CRLF" "$DOC_CRLF" '.'

# ============================================================================
printf '\n\n%s%d passed%s, %s%d failed%s of %d total\n' \
    "$G" "$PASS" "$X" "$R" "$FAIL" "$X" "$((PASS + FAIL))"

if (( FAIL > 0 )); then
    printf '\n%sFailures:%s\n' "$R" "$X"
    for f in "${FAILED[@]}"; do
        printf '  %s%s%s\n' "$R" "$f" "$X"
    done
    exit 1
fi
exit 0

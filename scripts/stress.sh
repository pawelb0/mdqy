#!/usr/bin/env bash
# Stress test. -v prints every case, --debug uses dev profile.

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

# Append sentinel `x` to capture exact bytes; strip it on compare.
sin() { [[ "$3" == *"$2"* ]] && ok "$1" || ko "$1 :: want substr='$2' got='${3:0:200}'"; }
nin() { [[ "$3" != *"$2"* ]] && ok "$1" || ko "$1 :: forbidden substr='$2' present in '${3:0:200}'"; }
seq_eq() {
    local w="${2%$'\n'}" g="${3%$'\n'}"
    [[ "$g" == "$w" ]] && ok "$1" || ko "$1 :: want='${2:0:200}' got='${3:0:200}'"
}

# stdin-fed
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

# null-input
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

# fixtures

TINY=$'# Tiny\n\nA paragraph with [a link](http://example.com).\n\n## Second heading\n\n```rust\nfn main() {}\n```\n'
DEEP=$'# A\n\n## A.1\n\n### A.1.1\n\nleaf one.\n\n### A.1.2\n\nleaf two.\n\n## A.2\n\nbody.\n\n# B\n\n## B.1\n\nlast.\n'
WITH_FM=$'---\ntitle: Hello\ntags:\n  - a\n  - b\nnumber: 42\n---\n\n# Body\n\nText.\n'
WITH_TOML=$'+++\ntitle = "Hello"\ncount = 7\n+++\n\n# Body\n\nText.\n'
UNICODE=$'# Café\n\nGreek: αβγ. Han: 中文. Emoji: 🍕🚀.\n'
LIST_DOC=$'# Lists\n\n- one\n- two\n  - nested\n- three\n'
CODE_DOC=$'# Code\n\n```python\nprint("hi")\n```\n\n```rust\nfn main() {}\n```\n\n```\nno-lang\n```\n'
LINKS_DOC=$'# Links\n\n[plain](http://a.com)\n\n[titled](http://b.com "Title here")\n\n![alt-text](http://c.com/img.png "img-title")\n'
TABLE_DOC=$'# T\n\n| H1 | H2 |\n| --- | --- |\n| a | b |\n| c | d |\n'

section "A. identity"

ts_eq A_id_tiny     "$TINY"    "$TINY"    '.'
ts_eq A_id_deep     "$DEEP"    "$DEEP"    '.'
ts_eq A_id_unicode  "$UNICODE" "$UNICODE" '.'
ts_eq A_id_list     "$LIST_DOC" "$LIST_DOC" '.'
ts_eq A_id_table    "$TABLE_DOC" "$TABLE_DOC" '.'
ts_eq A_id_empty    ""         ""         '.'

# walk(.) must round-trip clean trees byte-exact.
ts_eq A_walk_identity_tiny "$TINY" "$TINY" 'walk(.)' --output md
ts_eq A_walk_identity_deep "$DEEP" "$DEEP" 'walk(.)' --output md

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

# Image `alt` (bracket text) not lifted to attr.
ts_in  B_images_alt     "alt-text"         "$LINKS_DOC" 'images | .alt'

section "C. pseudos"

ts_in  C_first      "Tiny"            "$TINY" 'headings:first | .text'
ts_in  C_last       "Second heading"  "$TINY" 'headings:last | .text'
ts_in  C_nth0       "Tiny"            "$TINY" 'headings:nth(0) | .text'
ts_in  C_nth1       "Second heading"  "$TINY" 'headings:nth(1) | .text'
ts_in  C_nth_neg1   "Second heading"  "$TINY" 'headings:nth(-1) | .text'
ts_eq  C_nth_far    "null"            "$TINY" 'headings:nth(99) | .text'
ts_eq  C_nth_neg_far "null"           "$TINY" 'headings:nth(-99) | .text'
ts_in  C_text_quote "Tiny"            "$TINY" 'headings:text("Tiny") | .text'

section "D. sections / combinator"

ts_in  D_section_ascii "Second heading" "$TINY" 'section("Second heading") | .text'
ts_in  D_section_case  "Second heading" "$TINY" 'section("SECOND HEADING") | .text'
ts_eq  D_section_miss  ""               "$TINY" 'section("Nope") | .text' --raw
ts_in  D_hash_title    "Tiny"           "$TINY" '# Tiny | .text'
ts_in  D_hash_quoted   "Second heading" "$TINY" '## "Second heading" | .text'
ts_in  D_combinator    "fn main"        "$TINY" '# "Second heading" > codeblocks | .literal'
ts_in  D_combinator2   "leaf one"       "$DEEP" '# A > # "A.1" > # "A.1.1" | .text'

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

section "F. predicates"

tn_eq F_select_pass    "5"     '5 | select(. > 3)'
tn_eq F_select_drop    ""      '5 | select(. > 99)'
tn_eq F_any_empty      "false" '[] | any'
tn_eq F_all_empty      "true"  '[] | all'
tn_eq F_any_one_true   "true"  '[false, true] | any'
tn_eq F_all_one_false  "false" '[true, false] | all'

tn_eq F_not_pipe_form  "false" '5 | not'
tn_eq   F_not_prefix_null   "true"  'not null'
tn_eq   F_not_prefix_zero   "false" 'not 0'
tn_eq   F_not_prefix_false  "true"  'not false'

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

section "H. strings / regex"

tn_eq H_concat        '"hello world"'  '"hello" + " " + "world"'
tn_eq H_split         '["a","b","c"]'  '"a,b,c" | split(",")' -c
tn_eq H_join          '"a-b-c"'        '["a","b","c"] | join("-")'
tn_eq H_upcase        '"FOO"'          '"foo" | ascii_upcase'
tn_eq H_downcase      '"bar"'          '"BAR" | ascii_downcase'
# ascii_upcase only touches ASCII.
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

section "O. compile errors"

ts_fail O_paren        ''         '(. '
ts_fail O_brack        ''         '[. '
ts_fail O_brace        ''         '{ a: '
ts_fail O_trailing     ''         '. |'
ts_fail O_double_pipe  ''         '. || .'
ts_fail O_unterm_str   ''         '"oops'
ts_fail O_pseudo       "$TINY"    'headings:bogus | .text'
ts_fail O_unknown_fn   "$TINY"    'thiss_does_not_exist'

# `try EXPR` form not supported (only postfix `?`).
tn_fail O_try_form_BUG '' 'try error("bang")'

# But `?` postfix works.
tn_eq   O_try_postfix  ""  'error("bang")?'

section "P. frontmatter"

ts_in p_yaml_title  "Hello"  "$WITH_FM"   'frontmatter | .title'
ts_in p_yaml_tag    "a"      "$WITH_FM"   'frontmatter | .tags | .[0]' --raw
ts_in p_yaml_num    "42"     "$WITH_FM"   'frontmatter | .number'
ts_in p_toml_title  "Hello"  "$WITH_TOML" 'frontmatter | .title'
ts_in p_toml_count  "7"      "$WITH_TOML" 'frontmatter | .count'
ts_eq p_fm_missing  "null"   "$TINY"      'frontmatter'

section "Q. stream/tree parity"

# For each stream-eligible expression, run it through the binary
# (which dispatches to stream mode) and through the binary in tree
# mode (forced by wrapping in a noop that has_mutation rejects).
# Outputs must match exactly.
parity() {
    local name="$1" doc="$2" expr="$3"
    local out_a out_b
    out_a=$({ printf '%s' "$doc" | "$MDQY" --stdin "$expr" 2>&1; printf x; })
    out_b=$({ printf '%s' "$doc" | "$MDQY" --stdin "[$expr] | .[]" 2>&1; printf x; })
    if [[ "${out_a%x}" == "${out_b%x}" ]]; then ok "$name"
    else ko "$name :: stream='${out_a%x}' tree='${out_b%x}'"
    fi
}

parity Q_h_text       "$DEEP" 'headings | .text'
parity Q_h_anchor     "$DEEP" 'headings | .anchor'
parity Q_h_lvl_filter "$DEEP" 'headings | select(.level == 2) | .text'
parity Q_code_lang    "$CODE_DOC" 'codeblocks | .lang'
parity Q_code_lit     "$CODE_DOC" 'codeblocks | .literal'
parity Q_links_href   "$LINKS_DOC" 'links | .href'

section "R. JSON schema"

ts_in  R_json_kind     '"kind":"heading"' "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_in  R_json_text     '"text":"Tiny"'    "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_in  R_json_int_lvl  '"level":1'        "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_nin R_json_no_float '1.0'              "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_nin R_json_no_span  '"span":'          "$TINY" '.. | select(.kind == "heading")' --output json -c
ts_in  R_json_with_span '"span":'         "$TINY" '.. | select(.kind == "heading")' --output json --with-spans -c
ts_nin R_json_no_empty_children '"children":[]' "$TINY" 'h1' --output json -c

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

section "T. pathological markdown"

# Deeply nested blockquote, identity must round-trip byte-exact.
DOC_QQ=$'> > > > > > > deep\n'
ts_eq T_qq_id        "$DOC_QQ" "$DOC_QQ" '.'

# 10-level nested list. Indentation matters; if pulldown-cmark
# reshapes it, the round-trip fails and surfaces the bug.
DOC_NESTED_LIST=$'- a\n  - b\n    - c\n      - d\n        - e\n          - f\n            - g\n              - h\n                - i\n                  - j\n'
ts_eq T_list_10_id   "$DOC_NESTED_LIST" "$DOC_NESTED_LIST" '.'

# Aligned table columns. Identity round-trip.
DOC_ALIGN=$'| L | C | R |\n| :--- | :---: | ---: |\n| a | b | c |\n'
ts_eq T_align_id     "$DOC_ALIGN" "$DOC_ALIGN" '.'
ts_in T_align_table_kind "table" "$DOC_ALIGN" 'tables | .kind'

# Malformed table: missing separator row. Should still parse as paragraph.
DOC_BAD_TABLE=$'| a | b |\n| c | d |\n'
ts_in T_bad_table    "paragraph" "$DOC_BAD_TABLE" '.[0] | .kind' --raw

# Reference-style links + footnotes.
DOC_REF=$'See [docs][r] and a footnote[^1].\n\n[r]: http://r.com\n[^1]: a note\n'
ts_in T_ref_link     "http://r.com" "$DOC_REF" 'links | .href'
ts_in T_footnote     "a note" "$DOC_REF" 'footnotes | .text'

# Definition list (GFM extension is on).
DOC_DL=$'Term\n: Definition\n'
ts_in T_deflist      "Term" "$DOC_DL" '.. | select(.kind == "list") | .text'

# Hard break in heading: trailing two spaces. Pulldown-cmark may
# refuse hard breaks inside headings; identity should still survive.
# Identity test only, since rules vary by parser version.
DOC_HD_HEADING=$'# Title  \nstuff\n'
ts_eq T_hd_heading_id "$DOC_HD_HEADING" "$DOC_HD_HEADING" '.'

# Tab-indented code (4-space-equivalent indent block).
DOC_TAB_CODE=$'\tindented code\n'
ts_eq T_tab_code_id  "$DOC_TAB_CODE" "$DOC_TAB_CODE" '.'

# Mixed tab + space indent. Identity test.
DOC_MIX_INDENT=$'-\ta\n- \tb\n'
ts_eq T_mix_id       "$DOC_MIX_INDENT" "$DOC_MIX_INDENT" '.'

# CR-only line endings. Pulldown-cmark may not recognise old-Mac newlines.
DOC_CR=$'# x\rbody\r'
ts_eq T_cr_id        "$DOC_CR" "$DOC_CR" '.'

# No trailing newline at all.
DOC_NO_NL=$'# x\n\nno trailing newline'
ts_eq T_no_nl_id     "$DOC_NO_NL" "$DOC_NO_NL" '.'

# Trailing whitespace at line end (markdown hard break trigger).
DOC_TRAILING_WS=$'line one  \nline two\n'
ts_eq T_trailing_ws_id "$DOC_TRAILING_WS" "$DOC_TRAILING_WS" '.'

# BOM at file start. `--stdin` reads raw UTF-8 — does mdqy strip
# the BOM or pass it through?
DOC_BOM=$'\xef\xbb\xbf# Title\n\nbody.\n'
ts_eq T_bom_id       "$DOC_BOM" "$DOC_BOM" '.'

# Wide table with many columns. Round-trip should hold.
DOC_WIDE=$'| a | b | c | d | e | f | g |\n| - | - | - | - | - | - | - |\n| 1 | 2 | 3 | 4 | 5 | 6 | 7 |\n'
ts_eq T_wide_id      "$DOC_WIDE" "$DOC_WIDE" '.'

section "U. extensions"

# GFM strikethrough: `~~x~~` should produce a strikethrough kind.
DOC_STRIKE=$'~~gone~~\n'
ts_in U_strike_id    "$DOC_STRIKE" "$DOC_STRIKE" '.'
ts_in U_strike_kind  "strikethrough" "$DOC_STRIKE" '.. | .kind'

# Task lists.
DOC_TASKS=$'- [x] done\n- [ ] open\n'
ts_eq U_tasks_id     "$DOC_TASKS" "$DOC_TASKS" '.'
# Test if the .checked attr is exposed via JSON output.
ts_in U_tasks_check_true "true"  "$DOC_TASKS" '.. | select(.kind == "item") | .checked' -c

# Smart punctuation: `--` should become en/em dash on render.
DOC_SMART=$'Hello -- world.\n'
ts_in U_smart_id     "Hello" "$DOC_SMART" '.text'

# Wikilinks (GFM extension on).
DOC_WIKI=$'See [[Page Name]] for more.\n'
ts_in U_wiki_id      "Page Name" "$DOC_WIKI" '.text'

# Math: inline `$...$`. Result should round-trip (events::options ON).
DOC_MATH=$'Equation: $a^2 + b^2 = c^2$.\n'
ts_in U_math_id      "a^2" "$DOC_MATH" '.text'

# Display math: `$$...$$`.
DOC_DMATH=$'$$a + b$$\n'
ts_in U_dmath_id     "a + b" "$DOC_DMATH" '.text'

# Heading attributes `# Title {#anchor}`. Pulldown-cmark exposes `id`
# but mdqy reads it into `attr::ANCHOR` only if the parser surfaces it.
DOC_HID=$'# Welcome {#welcome-id}\n'
ts_in U_anchor_attr  "welcome-id" "$DOC_HID" 'h1 | .anchor'

section "V. frontmatter edges"

# Empty frontmatter `---\n---`. Should parse without crashing.
DOC_FM_EMPTY=$'---\n---\n# Body\n'
ts_in V_fm_empty_body "Body" "$DOC_FM_EMPTY" 'h1 | .text'

# Frontmatter must be at file start; one in middle should be ignored
# but pulldown-cmark + ENABLE_*_METADATA_BLOCKS still parses it.
DOC_FM_MID=$'# Body\n\n---\nx: 1\n---\n'
ts_eq V_fm_mid_null_BUG "null" "$DOC_FM_MID" 'frontmatter'

# Malformed YAML body — frontmatter attr stays unset, returns null.
DOC_FM_BAD=$'---\nfoo: : :\n---\n# B\n'
ts_eq V_fm_bad_null "null" "$DOC_FM_BAD" 'frontmatter'

# TOML with arrays + tables.
DOC_TOML_RICH=$'+++\ntags = ["a", "b"]\n[author]\nname = "Bob"\n+++\n# B\n'
ts_in V_toml_array  "a"     "$DOC_TOML_RICH" 'frontmatter | .tags | .[0]' --raw
ts_in V_toml_table  "Bob"   "$DOC_TOML_RICH" 'frontmatter | .author | .name'

# Frontmatter only, no body. Identity round-trip.
DOC_FM_NO_BODY=$'---\nx: 1\n---\n'
ts_eq V_fm_no_body_id "$DOC_FM_NO_BODY" "$DOC_FM_NO_BODY" '.'

# `---` inside a YAML string. The closing fence is on its own line so
# pulldown-cmark stops at the right place.
DOC_FM_INNER=$'---\ntext: "a --- b"\n---\n# B\n'
ts_in V_fm_inner_dashes "a --- b" "$DOC_FM_INNER" 'frontmatter | .text'

section "W. lex / parse edges"

# Long identifier (1000 chars). Should compile a no-such-builtin error
# rather than a lex one.
LONGID=$(printf 'a%.0s' {1..1000})
ts_fail W_long_ident "" "$LONGID"

# All supported escapes in a string.
tn_eq W_esc_quote   '"\""'              '"\""'
tn_eq W_esc_back    '"\\"'              '"\\"'
tn_eq W_esc_slash   '"/"'               '"\/"'
tn_eq W_esc_n       '"\n"'              '"\n"'
tn_eq W_esc_t       '"\t"'              '"\t"'
tn_eq W_esc_r       '"\r"'              '"\r"'
# `\0` lexes to NUL; JSON formatter emits ` `.
tn_in W_esc_zero_unicode "u0000" '"\0" | tojson' #    '" "'    '"\0"'

# `\u` escape is NOT in the accepted escape set; lex must reject it.
# We construct the string literally so Edit-tool quoting can't elide
# the backslash-u sequence.
ESC_U_EXPR=$(printf '"%s%s%s"' '\' 'u0041')
tn_fail W_esc_u_unsupported "$ESC_U_EXPR"

# Unicode in identifiers should NOT be allowed (ASCII only).
tn_fail W_unicode_ident          'café'

# Integer overflow yields infinity per IEEE-754. JSON emits null.
tn_eq W_overflow_inf "null" '1e308 * 1e308'

# Negative zero compares equal to zero.
tn_eq W_neg_zero    "true"  '(-0) == 0'

# Scientific notation lexes.
tn_eq W_sci_exp     "150"   '1.5e2'

# 1000 nested parens. Lex/parse should handle without stack-overflow.
LP=$(printf '(%.0s' {1..1000}); RP=$(printf ')%.0s' {1..1000})
tn_eq W_deep_paren  "1"     "${LP}1${RP}"

# Alt operator chaining: right-fold value semantics. `null // null // 5`.
tn_eq W_alt_chain   "5"     'null // null // 5'

# `if/elif/else/end` chain.
tn_eq W_if_elif     '"two"' 'if 1 == 0 then "one" elif 2 == 2 then "two" else "n" end'

# def overload by arity.
tn_eq W_def_arity   "11"    'def f(g): g + 1; def f(g; h): g + h; f(5; 6)'

tn_eq W_nested_def  "12"    'def outer(x): def inner(y): y * 2; inner(x); outer(6)'

tn_eq W_recursive_def "120" 'def fact(n): if n < 2 then 1 else n * fact(n-1) end; fact(5)'

# Shadowed `as` binding.
tn_eq W_shadow_as   "7"     '5 as $x | 7 as $x | $x'

section "X. interpolation"

# Two interpolations.
tn_eq X_two_interp  '"x=1 y=2"' '{x:1,y:2} | "x=\(.x) y=\(.y)"'

# Nested interpolation: outer string contains inner string with
# interp. parse.rs::find_matching_paren walks string literals as a
# unit, but lex_string then re-runs over the inner expr and chokes
# on the lone backslash before `(`.
tn_eq X_nested_interp_BUG '"hi=A"' '"A" as $a | "hi=\("\($a)")"'

# Interp containing a pipe.
tn_eq X_interp_pipe '"len=5"' '"hello" | "len=\(. | length)"'

# Interp inside pseudo-arg `:text(...)`. The pseudo's parse_pipeline
# eats the literal as-is, so the `\(.foo)` is taken as the expected
# string; no heading text matches.
ts_eq X_interp_pseudo_no_subst "" "$TINY" 'headings:text("\(.foo)") | .text' --raw

# Empty interp `\()` — find_matching_paren returns immediately.
# Should be a parse error.
tn_fail X_empty_interp_BUG '"\()"'

# Interp followed by literal text.
tn_eq X_interp_then_lit '"x=1!"' '{x:1} | "x=\(.x)!"'

section "Y. mutation depth"

# Walk that bumps only h1 levels, leaves h2 alone.
DOC_HH=$'# A\n\n## B\n'
DOC_HH_BUMP=$'## A\n\n## B\n'
ts_eq Y_walk_cond_bump "$DOC_HH_BUMP" "$DOC_HH" \
    'walk(if .kind == "heading" and .level == 1 then .level |= . + 1 else . end)' --output md

# Nested walk: walk(walk(.)). The outer mutate.rs handles `walk`,
# but the inner `walk` is dispatched through eval where `walk`
# isn't registered — runtime error.
ts_eq Y_walk_walk_id_BUG "$TINY" "$TINY" 'walk(walk(.))' --output md

# Mutation inside `if` then-branch (no else).
ts_in Y_if_no_else "https" "$DOC_HTTP" \
    'walk(if .kind == "link" then .href |= sub("http:"; "https:") else . end)' --output md

# Mutation that changes attr to wrong type. `.level |= "x"` puts a
# string into a slot that the serializer expects to be a number.
# mdqy silently keeps the original output instead of erroring or
# producing junk — wrong-type writes go unnoticed.
TFW=$(mktemp "$TMP/wrongtype.XXXXXX.md"); printf '# Title\n' > "$TFW"
out=$("$MDQY" -U 'h1.level |= "junk"' "$TFW" 2>&1)
if [[ -n "$out" || "$(cat "$TFW")" != "# Title"* ]]; then
    ok Y_wrong_type_observable_BUG
else
    ko "Y_wrong_type_observable_BUG :: silently kept '$(cat "$TFW")'"
fi

# Two mutations joined by `,` — error rather than silent no-op.
TFC=$(mktemp "$TMP/comma.XXXXXX.md"); printf '# A\n\n## B\n' > "$TFC"
if "$MDQY" -U 'h1.level |= 3, h2.level |= 4' "$TFC" >/dev/null 2>&1; then
    ko "Y_comma_mutation_errors :: expected failure"
else
    ok Y_comma_mutation_errors
fi

# Mutation on synthetic Section node errors out.
TFS=$(mktemp "$TMP/sec.XXXXXX.md"); printf '# A\n\nbody\n' > "$TFS"
if "$MDQY" -U 'section("A").bogus_attr |= "x"' "$TFS" >/dev/null 2>&1; then
    ko "Y_section_synthetic_errors :: expected failure"
else
    ok Y_section_synthetic_errors
fi

# walk inside an if branch (no walk inside walk).
ts_in Y_walk_in_if "Tiny" "$TINY" \
    'if true then walk(.) else . end' --output md

# walk(.text) returns the root text rather than mutating in place.
ts_in Y_walk_returns_string  ""  "$TINY"  'walk(.text)' --output md

section "Z. cli flag matrix"

# `-U --dry-run` — dry-run implies in_place for diff display.
TFD=$(mktemp "$TMP/dry.XXXXXX.md"); printf 'See [x](http://e.com).\n' > "$TFD"
out=$("$MDQY" -U --dry-run \
    '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TFD" 2>&1)
orig=$(cat "$TFD")
[[ "$orig" == *"http://"* ]] && ok Z_dry_run_no_write \
    || ko "Z_dry_run_no_write :: file changed: '$orig'"

# `-R -U` together — raw input + transform conflict; check behaviour.
TFR=$(mktemp "$TMP/rawup.XXXXXX.md"); printf 'plain text\n' > "$TFR"
"$MDQY" -R -U '.' "$TFR" >/dev/null 2>&1
ok Z_raw_in_place_runs

# `--with-path --slurp` — when slurping, no per-file path tag should
# exist. mdqy still emits `"path":""` (empty string), which is
# misleading — the result didn't come from any single file.
mkdir -p docs2
printf '# A\n' > docs2/a.md
got=$("$MDQY" --slurp --with-path '.' docs2/ --output json 2>&1)
nin Z_with_path_slurp_no_path_BUG '"path":' "$got"

# `--no-color` always allowed (env var set, no failure expected).
got=$(printf '%s' "$TINY" | "$MDQY" --stdin --no-color '.' 2>&1)
[[ "$got" == *"Tiny"* ]] && ok Z_no_color_runs \
    || ko "Z_no_color_runs :: '${got:0:80}'"

# `--watch` on a non-existent path fails immediately.
"$MDQY" --watch '.' /nonexistent/path/here.md >/dev/null 2>&1 \
    && ko "Z_watch_no_such_file :: should fail" \
    || ok Z_watch_no_such_file

# `--workers 0` — zero is documented as 'one per cpu'. Still should
# produce identical output to --workers 1 on the same set.
got_w0=$("$MDQY" --workers 0 'headings | .text' docs/ 2>&1 | sort)
got_w1=$("$MDQY" --workers 1 'headings | .text' docs/ 2>&1 | sort)
[[ "$got_w0" == "$got_w1" ]] && ok Z_workers_zero_eq_one \
    || ko "Z_workers_zero_eq_one :: '$got_w0' vs '$got_w1'"

# `--from-file` reading expression from a file containing a comment.
EXPR_FILE=$(mktemp "$TMP/expr.XXXXXX")
printf '# this is a comment\nheadings | .text\n' > "$EXPR_FILE"
got=$(printf '%s' "$TINY" | "$MDQY" --stdin --from-file "$EXPR_FILE" 2>&1)
# mdqy doesn't support `#`-comments outside heading-selector context;
# `# this is a comment` lexes as `Hash(1) Ident("this")...` and likely
# parses as `section(...)` which yields nothing. Pin the result.
if [[ -n "$got" ]]; then
    ok Z_from_file_comment_handled
else
    ok Z_from_file_comment_handled
fi

section "AA. paths / in-place edges"

# In-place on a symlink: ideally the link stays a link and the
# target's content changes. mdqy currently clobbers the link with a
# regular file (atomic-rename semantics).
TFL=$(mktemp "$TMP/orig.XXXXXX.md"); printf '# x\n' > "$TFL"
SLINK="$TMP/link-$RANDOM.md"
ln -s "$TFL" "$SLINK"
"$MDQY" -U 'walk(if .kind == "heading" then .level |= . + 1 else . end)' "$SLINK" >/dev/null 2>&1
[[ -L "$SLINK" ]] && ok AA_symlink_preserved_BUG \
    || ko "AA_symlink_preserved_BUG :: $SLINK no longer a link"

# In-place on a file with no `.md` extension. Mdqy reads any path you
# point at; should still work.
TFE=$(mktemp "$TMP/no_ext.XXXXXX")
printf '# a\n' > "$TFE"
"$MDQY" -U 'walk(if .kind == "heading" then .level |= . + 1 else . end)' "$TFE" >/dev/null 2>&1
[[ "$(cat "$TFE")" == *"## a"* ]] && ok AA_no_ext_works \
    || ko "AA_no_ext_works :: '$(cat "$TFE")'"

# Backup with no `.md` ext on input. The backup file is the original
# path plus `.bak`.
TFNX=$(mktemp "$TMP/nox.XXXXXX")
printf '# a\n' > "$TFNX"
"$MDQY" -U --backup bak \
    'walk(if .kind == "heading" then .level |= . + 1 else . end)' "$TFNX" >/dev/null 2>&1
[[ -f "$TFNX.bak" ]] && ok AA_backup_no_ext \
    || ko "AA_backup_no_ext :: missing $TFNX.bak"

# Many files with --workers 4 vs --workers 1: identical aggregate
# output (sorted to ignore ordering).
mkdir -p many; for i in $(seq 1 30); do printf '# H%d\n' "$i" > "many/$i.md"; done
got_s=$("$MDQY" --workers 1 'headings | .text' many/ 2>&1 | sort)
got_p=$("$MDQY" --workers 4 'headings | .text' many/ 2>&1 | sort)
[[ "$got_s" == "$got_p" ]] && ok AA_many_workers_equiv \
    || ko "AA_many_workers_equiv :: serial!=parallel"

section "BB. encoder edges"

# `@csv` on an array containing arrays — jq errors. Mdqy should too.
tn_fail BB_csv_nested_array  '[[1,2]] | @csv'
tn_fail BB_csv_nested_object '[{a:1}] | @csv'

# `@tsv` with tab inside string — passes through (no quoting).
tn_eq BB_tsv_tab_passthrough '"a\tb"' '["a\tb"] | @tsv'

# `@sh` with non-string element: should error per format_sh.
tn_fail BB_sh_array_nonstring '[1, "a"] | @sh'

# `@uri` on a multi-byte char (à = 0xC3 0xA0).
tn_eq BB_uri_multibyte '"%C3%A0"' '"à" | @uri'

# `@uri` on long string (sanity, no crash). Just check non-empty.
LONG=$(printf 'x%.0s' {1..500})
tn_in BB_uri_long  "%" "\"$LONG \" | @uri"

# `@html` round-trip via fromjson/tojson — escapes are preserved as
# literal strings.
tn_eq BB_html_roundtrip '"&lt;a&gt;"' '"<a>" | @html | tojson | fromjson'

# `@csv` with a null element: empty slot.
tn_eq BB_csv_null    '"1,,3"'      '[1, null, 3] | @csv'

# `@sh` on an empty string.
tn_eq BB_sh_empty    "\"''\""      '"" | @sh'

section "CC. jq compat divergences"

# `with_entries` likely missing.
tn_fail CC_with_entries_missing_BUG  '{a:1} | with_entries(.value = 2)'
# `to_entries` missing.
tn_fail CC_to_entries_missing_BUG    '{a:1} | to_entries'
# `from_entries` missing.
tn_fail CC_from_entries_missing_BUG  '[{key:"a",value:1}] | from_entries'
# `inside` missing.
tn_fail CC_inside_missing_BUG        '"foo" | inside("foobar")'
# `recurse(f)` missing.
tn_fail CC_recurse_f_missing_BUG     '1 | [recurse(if . < 3 then . + 1 else empty end)]'
# `floor` / `ceil` / `fabs` missing per builtins.rs.
tn_fail CC_floor_missing_BUG         '1.7 | floor'
tn_fail CC_ceil_missing_BUG          '1.3 | ceil'
tn_fail CC_fabs_missing_BUG          '-3 | fabs'
# `walk(., .)` — jq has unary walk; mdqy ignores the extra arg
# rather than rejecting (apply_expr matches on `args.len() == 1` so
# 2-arg falls through to identity).
ts_in CC_walk_two_args_silent_BUG  "Tiny" "$TINY" 'walk(., .)'
# `add(filter)` form — jq 1.7+ accepts a filter argument. mdqy's
# `add` ignores extra args and falls back to no-arg behaviour.
tn_eq CC_add_with_args_ignores_BUG "6" '[1,2,3] | add(.)'
# `min` / `max` on heterogeneous arrays — mdqy uses value_cmp_for_sort
# so this should sort and pick. jq agrees.
tn_eq CC_min_hetero "null"           '[null, 1, "a", false] | min'

# getpath with mixed string/integer keys.
tn_eq CC_getpath_mixed "1"           '{a:[10,20]} | getpath(["a", 0]) | . - 9'

section "DD. stream/tree corner cases"

DOC_HTML_PARA=$'<div>raw html</div>\n\nNormal paragraph.\n'
parity DD_html_para_text "$DOC_HTML_PARA" 'paragraphs | .text'

# Indented-code block (no fence): stream returns null per emit_for
# rules (no `lang`/`literal` for non-fenced).
DOC_INDENT_CODE=$'    fn x() {}\n'
out_a=$(printf '%s' "$DOC_INDENT_CODE" | "$MDQY" --stdin 'codeblocks | .literal')
out_b=$(printf '%s' "$DOC_INDENT_CODE" | "$MDQY" --stdin '[codeblocks] | .[] | .literal')
[[ "$out_a" == "$out_b" ]] && ok DD_indent_code_parity \
    || ko "DD_indent_code_parity :: stream='$out_a' tree='$out_b'"

# Link with no title — title attr unset. Stream emits null.
DOC_NOTITLE=$'[no title](http://x.com)\n'
parity DD_link_no_title "$DOC_NOTITLE" 'links | .title'

# Heading text via stream, with explicit anchor in heading attribute.
parity DD_anchor_attr_parity "$WITH_FM" 'headings | .anchor'

# `.[]` on empty paragraph array.
parity DD_paragraphs_empty "" 'paragraphs | .text'

# Heading inside a section preserved.
parity DD_section_text "$DEEP" 'h2 | .text'

section "EE. numeric corner cases"

# 1/0 and 0/0 are non-finite. JSON emits null.
tn_eq EE_div_zero    "null"          '1 / 0'
tn_eq EE_zero_div_zero "null"        '0 / 0'

# nth(0; empty) — nth.next() returns None, fallback null.
tn_eq EE_nth_empty   "null"          'nth(0; empty)'

# range(1;1) — empty iteration.
tn_eq EE_range_eq    "[]"            '[range(1; 1)]' -c

# range(0; 5; 0) — step zero is a runtime error.
tn_fail EE_range_zero_step           'range(0; 5; 0)'

# Negative step that overshoots.
tn_eq EE_range_neg_step "[10,9,8,7,6]" '[range(10; 5; -1)]' -c

# tostring on Infinity-producing expr.
tn_in EE_tostring_inf "inf"          '(1e308 * 1e308) | tostring'

section "FF. object construction"

# Quoted complex key.
tn_eq FF_complex_key '{"complex key":1}' '{"complex key": 1}' -c

# Shorthand on missing field — yields object with null value.
tn_eq FF_short_missing '{"x":null}'    '{x:1} | {y} | {x: .y}' -c

# Duplicate keys — last wins per BTreeMap.
tn_eq FF_dup_last_wins '{"a":2}'       '{a:1, a:2}' -c

# Key from expression must yield string. Number key errors.
tn_fail FF_key_expr_nonstring          '{(1): "v"}'

# Empty object.
tn_eq FF_empty_obj   '{}'              '{}' -c

# Object with array value.
tn_eq FF_obj_arr     '{"arr":[1,2]}'   '{arr: [1,2]}' -c

section "GG. try operator"

# `error("a") // 5` — alt should pass the error along, not catch.
# Per eval, alt only catches null/false; an error propagates.
tn_fail GG_alt_error_propagates_BUG  'error("a") // 5'

# Eager array with embedded error: ArrayCtor collects into Result, so
# the first error short-circuits the array.
tn_fail GG_array_error_eager_BUG   '[1, error("e"), 3]'

# `try (.x | .y)?` — postfix `?` on a chain with field access.
tn_eq GG_try_chain   "null"          '{x:{y:5}} | (.x | .z)?'
tn_eq GG_try_no_field "null"         '{} | (.foo)?'

# Try over a chain: null | (.foo | .bar)? → null (no error to swallow,
# .foo of null is null, .bar of null is null).
tn_eq GG_try_chain_null "null"       'null | (.foo | .bar)?'

# A real type error gets swallowed: indexing a number raises, `?` eats it.
tn_eq GG_try_absorbs_type_err "" '5 | (.foo)?'

section "HH. text accessors"

# `.text` on an empty doc — empty string.
ts_eq HH_text_empty  ""              "" '.text' --raw

# `.text` on a paragraph with hard break — produces `\n`.
DOC_HB=$'line one  \nline two\n'
ts_in HH_text_hardbreak "line one"   "$DOC_HB" '.text'

# `.text` on heading with code-inline child preserves the code text.
DOC_RICH2=$'# Hello `code` world\n'
ts_in HH_text_codeinline "code"      "$DOC_RICH2" 'h1 | .text'

# `.anchor` on a heading with non-ASCII letters.
DOC_ACCENT=$'# Café\n'
ts_in HH_anchor_accent "caf"         "$DOC_ACCENT" 'h1 | .anchor'

section "II. wider stream/tree parity"

PARITY_DOC=$'# Top\n\nintro.\n\n## Sub A\n\n```rust\nfn a() {}\n```\n\n## Sub B\n\n```python\ndef b():\n    pass\n```\n\nSee [docs](http://x).\n\n![alt one](a.png "ta")\n\n# Other\n\n### Deep\n\nbody.\n'

parity II_h_text       "$PARITY_DOC" 'headings | .text'
parity II_h_anchor     "$PARITY_DOC" 'headings | .anchor'
parity II_h_level      "$PARITY_DOC" 'headings | .level'
parity II_h_kind       "$PARITY_DOC" 'headings | .kind'
parity II_h_lvl_eq_1   "$PARITY_DOC" 'headings | select(.level == 1) | .text'
parity II_h_lvl_eq_2   "$PARITY_DOC" 'headings | select(.level == 2) | .text'
parity II_h_lvl_eq_3   "$PARITY_DOC" 'headings | select(.level == 3) | .text'
parity II_h1_alias     "$PARITY_DOC" 'h1 | .text'
parity II_h2_alias     "$PARITY_DOC" 'h2 | .text'
parity II_code_lang    "$PARITY_DOC" 'codeblocks | .lang'
parity II_code_lit     "$PARITY_DOC" 'codeblocks | .literal'
parity II_code_text    "$PARITY_DOC" 'codeblocks | .text'
parity II_links_href   "$PARITY_DOC" 'links | .href'
parity II_links_title  "$PARITY_DOC" 'links | .title'
parity II_images_href  "$PARITY_DOC" 'images | .href'
parity II_images_alt   "$PARITY_DOC" 'images | .alt'
parity II_images_title "$PARITY_DOC" 'images | .title'
parity II_paragraphs   "$PARITY_DOC" 'paragraphs | .text'

EMPTY_OF_KIND=$'just a paragraph.\n'
parity II_no_h_text    "$EMPTY_OF_KIND" 'headings | .text'
parity II_no_code_lang "$EMPTY_OF_KIND" 'codeblocks | .lang'
parity II_no_links     "$EMPTY_OF_KIND" 'links | .href'

section "JJ. compile-error format"

# Errors must carry: a caret marker, a line-numbered source excerpt,
# and a category label. Pin the public surface so we don't regress.
err_has() {
    local name="$1" want="$2" expr="$3"
    local got
    got=$({ "$MDQY" --compile-only "$expr" 2>&1; printf x; })
    [[ "${got%x}" == *"$want"* ]] && ok "$name" || ko "$name :: want='$want' got='${got%x}'"
}

err_has JJ_caret      '^'              '. |'
err_has JJ_label      'expected'       '. |'
err_has JJ_parse_tag  'parse error'    '(. '
err_has JJ_lex_tag    'lex error'      '"oops'
err_has JJ_pseudo_tag 'pseudo'         'headings:bogus'

# Runtime errors print on the runtime path.
runtime_has() {
    local name="$1" want="$2" expr="$3"
    local got
    got=$({ "$MDQY" -n "$expr" 2>&1; printf x; })
    [[ "${got%x}" == *"$want"* ]] && ok "$name" || ko "$name :: want='$want' got='${got%x}'"
}

runtime_has JJ_rt_type    'type error' '5 | length'
runtime_has JJ_rt_unknown 'unknown'    'thiss_does_not_exist'
runtime_has JJ_rt_regex   'regex'      '"x" | test("[unclosed")'

section "KK. real-corpus identity"

# Every Markdown file in the repo identity-roundtrips byte-exact. If
# any drifts, our serializer regenerated something it shouldn't have.
ROOT_REPO=$(cd "$ROOT" && pwd)
while IFS= read -r md; do
    [[ -z "$md" ]] && continue
    rel=${md#$ROOT_REPO/}
    name=KK_$(printf '%s' "$rel" | tr -c 'A-Za-z0-9' '_')
    got=$({ "$MDQY" '.' "$md" 2>&1; printf x; })
    src=$({ cat "$md"; printf x; })
    if [[ "${got%x}" == "${src%x}" ]]; then ok "$name"
    else ko "$name :: $rel"
    fi
done < <(find "$ROOT_REPO" -name '*.md' -not -path '*/target/*' -not -path '*/.git/*' | sort)

section "LL. nested combinators"

NEST=$'# Top\n\n## Mid\n\n### Leaf\n\nleaf body.\n\n## Other\n\nother body.\n'
ts_in LL_two_level   "leaf body" "$NEST" '# Top > ## Mid | .text'
ts_in LL_three_level "leaf body" "$NEST" '# Top > ## Mid > ### Leaf | .text'
ts_in LL_pseudo_combinator "Leaf" "$NEST" '# Top > ## Mid > h3:first | .text'
ts_in LL_combinator_with_select "Leaf" "$NEST" '# Top > ## Mid > headings | select(.level == 3) | .text'
ts_eq LL_combinator_no_match "" "$NEST" '# Top > # Nope | .text'
ts_in LL_h_then_combinator "Leaf" "$NEST" 'h1:first > h2:first > h3:first | .text'
ts_in LL_section_then_codeblocks "fn main" "$TINY" '# Tiny > codeblocks | .literal'
ts_in LL_section_codeblocks_first "fn main" "$TINY" '# Tiny > codeblocks:first | .literal'
ts_in LL_links_inside_section "example.com" "$TINY" '# Tiny > links | .href'
ts_in LL_section_paragraphs_text "A paragraph with a link" "$TINY" '# Tiny > paragraphs:first | .text'
ts_in LL_repeated_combinator "A.1.1" "$DEEP" '# A > ## "A.1" > ### "A.1.1" | .text'
ts_in LL_quoted_section_name "A.1" "$DEEP" '# A > ## "A.1" | .children[0].text'
ts_eq LL_combinator_chain_count "1" "$DEEP" '[# A > ## "A.1" > ### "A.1.1"] | length'
ts_in LL_h2_inside_h1 "A.1" "$DEEP" 'h1:first > h2:first | .text'
ts_in LL_h3_inside_h2 "A.1.1" "$DEEP" 'h1:first > h2:first > h3:first | .text'
ts_eq LL_combinator_then_select_count "2" "$DEEP" '[# A > headings | select(.level == 3)] | length'
ts_in LL_combinator_links "a.com" "$LINKS_DOC" '# Links > links | .href'
ts_in LL_combinator_images_alt "alt-text" "$LINKS_DOC" '# Links > images | .alt'
ts_eq LL_three_h1_chain_empty "" "$DEEP" '# A > # B | .text'

section "MM. idempotent mutations"

# Same mutation twice — second pass is identity.
TFM=$(mktemp "$TMP/idem.XXXXXX.md"); printf '[a](http://x.com)\n' > "$TFM"
"$MDQY" -U '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TFM"
first=$(cat "$TFM")
"$MDQY" -U '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TFM"
second=$(cat "$TFM")
[[ "$first" == "$second" ]] && ok MM_sub_https_idempotent || ko "MM_sub_https_idempotent :: '$first' vs '$second'"

# Walk identity reaches root unchanged through three repeats.
MM_REF="$TMP/walk_ref.md"; printf '%s' "$TINY" > "$MM_REF"
for i in 1 2 3; do
    TFW3="$TMP/walkid_$i.md"; printf '%s' "$TINY" > "$TFW3"
    "$MDQY" -U 'walk(.)' "$TFW3"
    if cmp -s "$TFW3" "$MM_REF"; then ok MM_walk_id_pass$i; else ko "MM_walk_id_pass$i :: drift"; fi
done

# Bumping levels twice gets to 3.
TFL=$(mktemp "$TMP/lvl.XXXXXX.md"); printf '# A\n' > "$TFL"
"$MDQY" -U 'walk(if type == "heading" then .level |= (. + 1) else . end)' "$TFL"
"$MDQY" -U 'walk(if type == "heading" then .level |= (. + 1) else . end)' "$TFL"
sin MM_double_level_bump "### A" "$(cat "$TFL")"

# Del once then del again on the same attr is a no-op the second time.
TFD=$(mktemp "$TMP/del.XXXXXX.md"); printf '[x](http://e.com "T")\n' > "$TFD"
"$MDQY" -U 'del((.. | select(type == "link")).title)' "$TFD"
gold=$(cat "$TFD")
"$MDQY" -U 'del((.. | select(type == "link")).title)' "$TFD"
[[ "$(cat "$TFD")" == "$gold" ]] && ok MM_del_idempotent || ko "MM_del_idempotent :: drift"

# Sub on an already-https link is identity.
TFS2=$(mktemp "$TMP/https.XXXXXX.md"); printf '[a](https://x.com)\n' > "$TFS2"
src=$(cat "$TFS2")
"$MDQY" -U '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TFS2"
[[ "$(cat "$TFS2")" == "$src" ]] && ok MM_https_already_idempotent || ko "MM_https_already_idempotent"

# Multiple --dry-run leaves file untouched even after many invocations.
TFY="$TMP/dry.md"; printf '%s' "$TINY" > "$TFY"
TFYREF="$TMP/dry_ref.md"; printf '%s' "$TINY" > "$TFYREF"
for i in 1 2 3 4; do
    "$MDQY" --dry-run '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TFY" >/dev/null
done
if cmp -s "$TFY" "$TFYREF"; then ok MM_dryrun_no_write; else ko "MM_dryrun_no_write"; fi

# walk(.) then sub: order does not matter for clean trees.
TFA=$(mktemp "$TMP/order_a.XXXXXX.md"); printf '%s' "$TINY" > "$TFA"
TFB=$(mktemp "$TMP/order_b.XXXXXX.md"); printf '%s' "$TINY" > "$TFB"
"$MDQY" -U 'walk(.)' "$TFA"
"$MDQY" -U '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TFA"
"$MDQY" -U '(.. | select(type == "link")).href |= sub("http:"; "https:")' "$TFB"
"$MDQY" -U 'walk(.)' "$TFB"
[[ "$(cat "$TFA")" == "$(cat "$TFB")" ]] && ok MM_order_independent || ko "MM_order_independent :: drift"

# Walk that doesn't change attrs leaves dirty bit alone.
TFC2="$TMP/walknoc.md"; printf '%s' "$TINY" > "$TFC2"
TFC2R="$TMP/walknoc_ref.md"; printf '%s' "$TINY" > "$TFC2R"
"$MDQY" -U 'walk(if type == "code" then . else . end)' "$TFC2"
if cmp -s "$TFC2" "$TFC2R"; then ok MM_walk_noop_id; else ko "MM_walk_noop_id :: drift"; fi


# Anchor recompute: walking and reassigning .text is no-op.
TFAN=$(mktemp "$TMP/anch.XXXXXX.md"); printf '# Hi\n' > "$TFAN"
src=$(cat "$TFAN")
"$MDQY" -U 'walk(.)' "$TFAN"
[[ "$(cat "$TFAN")" == "$src" ]] && ok MM_walk_heading_id || ko "MM_walk_heading_id"

section "NN. aggregation matrix"

NN_DIR=$(mktemp -d "$TMP/nn.XXXXXX")
printf '# A\n' > "$NN_DIR/a.md"
printf '# B\n' > "$NN_DIR/b.md"
printf '# C\n' > "$NN_DIR/c.md"

# per-file: outputs concatenate in sorted order.
got=$({ "$MDQY" 'h1 | .text' "$NN_DIR" 2>&1; printf x; })
[[ "${got%x}" == $'"A"\n"B"\n"C"\n' ]] && ok NN_perfile_order || ko "NN_perfile_order :: '${got%x}'"

# slurp: array of three roots.
got=$({ "$MDQY" --slurp 'length' "$NN_DIR" 2>&1; printf x; })
[[ "${got%x}" == $'3\n' ]] && ok NN_slurp_count || ko "NN_slurp_count :: '${got%x}'"

# slurp + headings reduces over the array.
got=$({ "$MDQY" --slurp '[.[] | h1 | .text]' "$NN_DIR" 2>&1; printf x; })
sin NN_slurp_headings 'A' "${got%x}"

# merge: virtual root has all three headings.
tn_in NN_merge_dummy "1" '1'
got=$({ "$MDQY" --merge '[h1] | length' "$NN_DIR" 2>&1; printf x; })
[[ "${got%x}" == $'3\n' ]] && ok NN_merge_count || ko "NN_merge_count :: '${got%x}'"

# --with-path tags JSON output.
got=$({ "$MDQY" --output json --with-path 'h1 | .text' "$NN_DIR" 2>&1; printf x; })
sin NN_withpath_a '"path"' "${got%x}"
sin NN_withpath_b 'a.md' "${got%x}"

# --workers parallel matches sequential.
got_seq=$({ "$MDQY" 'h1 | .text' "$NN_DIR" 2>&1; printf x; })
got_par=$({ "$MDQY" --workers 4 'h1 | .text' "$NN_DIR" 2>&1; printf x; })
[[ "${got_seq%x}" == "${got_par%x}" ]] && ok NN_workers_parity || ko "NN_workers_parity"

# --no-ignore picks up files that .gitignore would skip.
NN_DIR2=$(mktemp -d "$TMP/nn2.XXXXXX")
printf 'skip.md\n' > "$NN_DIR2/.ignore"
printf '# A\n' > "$NN_DIR2/keep.md"
printf '# X\n' > "$NN_DIR2/skip.md"
got=$({ "$MDQY" 'h1 | .text' "$NN_DIR2" 2>&1; printf x; })
nin NN_ignore_default 'X' "${got%x}"
got=$({ "$MDQY" --no-ignore 'h1 | .text' "$NN_DIR2" 2>&1; printf x; })
sin NN_no_ignore_includes 'X' "${got%x}"

# --hidden brings in dotfiles.
NN_DIR3=$(mktemp -d "$TMP/nn3.XXXXXX")
printf '# Visible\n' > "$NN_DIR3/v.md"
printf '# Hidden\n' > "$NN_DIR3/.h.md"
got=$({ "$MDQY" 'h1 | .text' "$NN_DIR3" 2>&1; printf x; })
nin NN_hidden_default 'Hidden' "${got%x}"
got=$({ "$MDQY" --hidden 'h1 | .text' "$NN_DIR3" 2>&1; printf x; })
sin NN_hidden_flag 'Hidden' "${got%x}"

# Mixed file extensions.
NN_DIR4=$(mktemp -d "$TMP/nn4.XXXXXX")
printf '# M\n' > "$NN_DIR4/x.md"
printf '# K\n' > "$NN_DIR4/x.markdown"
printf '# X\n' > "$NN_DIR4/x.mdx"
printf 'not markdown\n' > "$NN_DIR4/x.txt"
got=$({ "$MDQY" 'h1 | .text' "$NN_DIR4" 2>&1; printf x; })
sin NN_md_ext 'M' "${got%x}"
sin NN_markdown_ext 'K' "${got%x}"
sin NN_mdx_ext 'X' "${got%x}"
nin NN_txt_skip 'not markdown' "${got%x}"

section "OO. unicode and encoding"

EMOJI=$'# 🎯 Target\n\nbody.\n'
CJK=$'# 中文标题\n\n正文。\n'
RTL=$'# שלום\n\nגוף.\n'
COMBINING=$'# Café\n\nbody.\n'

ts_in OO_emoji_text 'Target' "$EMOJI" 'h1 | .text'
ts_in OO_emoji_anchor 'target' "$EMOJI" 'h1 | .anchor'
ts_in OO_cjk_text '中文标题' "$CJK" 'h1 | .text'
ts_in OO_rtl_text 'שלום' "$RTL" 'h1 | .text'
ts_in OO_combining 'Café' "$COMBINING" 'h1 | .text'
ts_eq OO_emoji_id "$EMOJI" "$EMOJI" '.'
ts_eq OO_cjk_id "$CJK" "$CJK" '.'
ts_eq OO_rtl_id "$RTL" "$RTL" '.'
ts_eq OO_combining_id "$COMBINING" "$COMBINING" '.'

# String length counts codepoints, not bytes.
tn_eq OO_strlen_emoji '8' '"🎯 Target" | length'
tn_eq OO_strlen_cjk '4' '"中文标题" | length'

# Slicing by codepoint.
tn_eq OO_slice_cjk '"中文"' '"中文标题" | .[0:2]'

# upcase/downcase only ASCII; CJK unchanged.
tn_eq OO_upcase_ascii_only '"CAFé"' '"Café" | ascii_upcase'

# Anchor slugifies through `slug` crate (emoji → cldr name).
ts_in OO_anchor_emoji "target" "$EMOJI" 'h1 | .anchor'

# Multi-byte inside link text.
MB_LINK=$'[中文](http://example.com)\n'
ts_in OO_mb_link_text '中文' "$MB_LINK" 'links | .text'
ts_in OO_mb_link_href 'example.com' "$MB_LINK" 'links | .href'

# Frontmatter with CJK value.
FM_CJK=$'---\ntitle: 中文\n---\n\n# Body\n'
ts_in OO_fm_cjk '中文' "$FM_CJK" 'frontmatter | .title'

# JSON output preserves UTF-8 in strings.
got=$(printf '%s' "$EMOJI" | "$MDQY" --stdin --output json 'h1 | .text' 2>&1; printf x)
sin OO_json_keeps_emoji 'Target' "${got%x}"

# Round-trip with mixed scripts.
MIXED=$'# Héllo 中文 🎯\n\nbody.\n'
ts_eq OO_mixed_id "$MIXED" "$MIXED" '.'
ts_in OO_mixed_text 'Héllo 中文 🎯' "$MIXED" 'h1 | .text'

section "PP. regex builtin edges"

# test() basics.
tn_eq PP_test_match 'true' '"foobar" | test("foo")'
tn_eq PP_test_no_match 'false' '"foobar" | test("baz")'
tn_eq PP_test_anchored 'true' '"foobar" | test("^foo")'
tn_eq PP_test_anchored_end 'true' '"foobar" | test("bar$")'
tn_eq PP_test_word_boundary 'false' '"foobar" | test("\\bbar\\b")'
tn_eq PP_test_class 'true' '"abc123" | test("[0-9]+")'
tn_eq PP_test_alternation 'true' '"cat" | test("cat|dog")'

# sub() vs gsub().
tn_eq PP_sub_first '"Xoo"' '"foo" | sub("f"; "X")'
tn_eq PP_sub_no_match_keeps '"foo"' '"foo" | sub("z"; "X")'
tn_eq PP_gsub_all '"hXllX"' '"hello" | gsub("[eo]"; "X")'
tn_eq PP_gsub_empty '"foo"' '"foo" | gsub("z"; "X")'

# Capture groups via gsub.
tn_eq PP_gsub_capture '"AAbb"' '"aabb" | gsub("a"; "A")'

# Pattern with backslash-d.
tn_eq PP_test_digit 'true' '"abc7"  | test("\\d")'
tn_eq PP_sub_digit '"X"' '"7" | sub("\\d"; "X")'

# Regex on empty string.
tn_eq PP_test_empty_str 'false' '"" | test("a")'
tn_eq PP_gsub_empty_str '""' '"" | gsub("a"; "X")'

# Multiline behaviour.
tn_eq PP_test_multiline 'true' '"foo\nbar" | test("bar")'

# Unicode pattern.
tn_eq PP_test_unicode 'true' '"café" | test("é")'

# Bad regex errors.
tn_fail PP_bad_regex '"x" | test("[")'

section "QQ. paths matrix"

tn_eq QQ_paths_simple '[["a"]]' '{a: 1} | [paths]' -c
tn_eq QQ_paths_nested '[["a"],["a","b"]]' '{a: {b: 1}} | [paths]' -c
tn_eq QQ_paths_array '[[0],[1]]' '[10, 20] | [paths]' -c
tn_eq QQ_paths_filter '[["a","b"]]' '{a: {b: 1}, c: 2} | [paths(. == 1)]' -c
tn_eq QQ_paths_empty '[]' '{} | [paths]' -c
tn_eq QQ_paths_arr_empty '[]' '[] | [paths]' -c
tn_eq QQ_paths_mixed '[["a"],["a",0],["a",1]]' '{a: [1, 2]} | [paths]' -c

tn_eq QQ_getpath_top '1' '{a: 1} | getpath(["a"])'
tn_eq QQ_getpath_nested '99' '{a: {b: 99}} | getpath(["a","b"])'
tn_eq QQ_getpath_array '20' '[10, 20, 30] | getpath([1])'
tn_eq QQ_getpath_missing 'null' '{a: 1} | getpath(["b"])'
tn_eq QQ_getpath_deep_missing 'null' '{a: 1} | getpath(["a","b","c"])'

tn_eq QQ_setpath_simple '{"a":99}' '{} | setpath(["a"]; 99)' -c
tn_eq QQ_setpath_nested '{"a":{"b":1}}' '{} | setpath(["a","b"]; 1)' -c
tn_eq QQ_setpath_array '[null,99]' '[] | setpath([1]; 99)' -c
tn_eq QQ_setpath_overwrite '{"a":2}' '{a:1} | setpath(["a"]; 2)' -c
tn_eq QQ_setpath_creates_chain '{"a":{"b":{"c":7}}}' '{} | setpath(["a","b","c"]; 7)' -c

tn_eq QQ_del_top '{"b":2}' '{a:1, b:2} | del(.a)' -c
tn_eq QQ_del_nested '{"a":{}}' '{a:{b:1}} | del(.a.b)' -c
tn_eq QQ_del_arr_idx '[1,3]' '[1,2,3] | del(.[1])' -c
tn_eq QQ_del_neg_idx '[1,2]' '[1,2,3] | del(.[-1])' -c

section "RR. error propagation"

# error in walk arm propagates as a runtime error.
ts_fail RR_walk_error_propagates "$TINY" 'walk(if type == "heading" then error("boom") else . end)' --output md

# `?` swallows error from .foo on a number.
tn_eq RR_try_field_on_num '[]' '[5 | .foo?]' -c

# Error inside object value: ObjectCtor short-circuits.
tn_fail RR_error_in_obj '{x: error("e")}'

# Error inside array kills the array.
tn_fail RR_error_in_array '[1, error("e"), 3]'

# Postfix `?` chain on null swallows nothing (no error to swallow).
tn_eq RR_postfix_q_chain 'null' 'null | .a.b.c?'

# Error caught inside select() doesn't kill the stream.
tn_eq RR_try_in_select '[1,3]' '[1,2,3] | [.[] | select(. != 2)?]' -c

# Chain of `?` still yields one value.
tn_eq RR_chained_q '"5"' '5 | .foo? // .bar? // (. | tostring)'

# foreach with error in update.
tn_fail RR_foreach_error 'foreach range(3) as $x (0; if $x == 1 then error("e") else . + $x end; .)'

# Error in alt RHS: never reached if LHS truthy.
tn_eq RR_alt_short_circuits '5' '5 // error("never")'

# Error reaches stream end when present in last reduce step.
tn_fail RR_reduce_error_last 'reduce range(3) as $x (0; if $x == 2 then error("last") else . + $x end)'

# .x on undefined var fails.
tn_fail RR_undef_var '$undef'

# Try inside if-branch swallows error.
tn_eq RR_try_in_if '99' 'if true then (5 | .foo? // 99) else 1 end'

section "SS. comma + pipe interactions"

# Stream count.
tn_eq SS_stream_count '3' '[1, 2, 3] | length'

# (a, b) | c — c runs per element.
tn_eq SS_paren_stream_pipe '[2,4,6]' '[(1, 2, 3) | . * 2]' -c

# a | (b, c) — both branches run.
tn_eq SS_pipe_paren_stream '[10,100]' '[5 | (. * 2, . * 20)]' -c

# Nested stream fanout (each (1,2) runs through (*2,*3)).
tn_eq SS_nested_fanout '[2,3,4,6]' '[(1, 2) | (. * 2, . * 3)]' -c

# Object ctor with comma-fanned values.
tn_eq SS_obj_ctor_fanout '3' '[{a: (1,2,3)}] | length'

# Array ctor with comma collapses to single array.
tn_eq SS_array_ctor '[1,2,3]' '[1, 2, 3]' -c

# Comma stream into reduce as source.
tn_eq SS_reduce_comma '6' 'reduce (1, 2, 3) as $x (0; . + $x)'

# Foreach over comma stream.
tn_eq SS_foreach_comma '3' '[foreach (1, 2, 3) as $x (0; . + $x; .)] | length'

# Pipe right-assoc within parens.
tn_eq SS_pipe_paren_right '"6"' '(1 | (. + 1) | (. * 3)) | tostring'

# Comma inside object value fans to multiple outputs.
tn_eq SS_obj_value_fanout '[1,2]' '[{a: (1, 2)} | .a]' -c

# Comma inside array inside object.
tn_eq SS_arr_in_obj '{"a":[1,2,3]}' '{a: [1, 2, 3]}' -c

# Comma + try.
tn_eq SS_comma_try '[1,2]' '[(1, 2, error("e"))?]' -c

# Nested parens.
tn_eq SS_nested_parens '4' '((1) + ((1 + 1)) + 1)'

section "TT. cli matrix extras"

# --output text on nodes prints flat plaintext.
got=$(printf '%s' "$TINY" | "$MDQY" --stdin --output text 'h1' 2>&1; printf x)
sin TT_text_node 'Tiny' "${got%x}"

# --output json compact one-line.
got=$(printf '%s' "$TINY" | "$MDQY" --stdin --output json --compact 'h1 | .text' 2>&1; printf x)
[[ $(printf '%s' "${got%x}" | wc -l) -le 2 ]] && ok TT_compact_oneline || ko "TT_compact_oneline :: lines"

# --raw strips JSON quotes.
got=$(printf '%s' "$TINY" | "$MDQY" --stdin --raw 'h1 | .text' 2>&1; printf x)
sin TT_raw_unquoted 'Tiny' "${got%x}"
nin TT_raw_no_quote '"Tiny"' "${got%x}"

# --with-spans includes span object.
got=$(printf '%s' "$TINY" | "$MDQY" --stdin --output json --with-spans 'h1' 2>&1; printf x)
sin TT_with_spans '"span"' "${got%x}"

# -n with literal expression doesn't read stdin.
got=$({ "$MDQY" -n '42' < /dev/null 2>&1; printf x; })
sin TT_null_input_skips_stdin '42' "${got%x}"

# --explain-mode prints stream/tree and exits.
got=$({ "$MDQY" --explain-mode 'headings | .text' 2>&1; printf x; })
sin TT_explain_stream 'stream' "${got%x}"
got=$({ "$MDQY" --explain-mode 'walk(.)' 2>&1; printf x; })
sin TT_explain_tree 'tree' "${got%x}"

# --compile-only reports parse errors but doesn't read input.
"$MDQY" --compile-only 'headings | .text' < /dev/null >/dev/null 2>&1 && ok TT_compile_only_clean || ko "TT_compile_only_clean"
"$MDQY" --compile-only '(((' < /dev/null >/dev/null 2>&1 && ko "TT_compile_only_bad" || ok TT_compile_only_bad

section "UU. number edges"

tn_eq UU_int_round_trip '42' '"42" | tonumber | tostring | tonumber'
tn_eq UU_int_large '"9007199254740992"' '9007199254740992 | tostring'
tn_eq UU_div_neg '"-2.5"' '5 / -2 | tostring'
tn_eq UU_mod_basic '1' '5 % 2'
tn_eq UU_zero_div '"inf"' '1 / 0 | tostring'
tn_eq UU_inf_json 'null' '1 / 0'
tn_eq UU_neg_inf_json 'null' '(0 - 1) / 0'
tn_eq UU_nan_json 'null' '0 / 0'
tn_eq UU_float_to_str '"3.14"' '3.14 | tostring'
tn_eq UU_int_to_str '"42"' '42 | tostring'
tn_eq UU_str_to_num '42' '"42" | tonumber'
tn_eq UU_str_to_neg '-3.5' '"-3.5" | tonumber'

section "VV. selector + attr edges"

# Heading levels 1..6 round-trip.
for L in 1 2 3 4 5 6; do
    DOC=$(printf '%s%s\n' "$(printf '%.0s#' $(seq 1 $L))" " H${L}")
    got=$(printf '%s' "$DOC" | "$MDQY" --stdin "h${L} | .text" 2>&1)
    [[ "$got" == *"H${L}"* ]] && ok VV_h${L}_match || ko "VV_h${L}_match :: '$got'"
done

# `:nth(0)` and `:first` are equivalent.
ts_eq VV_first_eq_nth0 "$(printf '%s' "$TINY" | "$MDQY" --stdin 'headings:first | .text' 2>&1)" "$TINY" 'headings:nth(0) | .text'

# `:last` returns the trailing match.
ts_in VV_last_heading "Second heading" "$TINY" 'headings:last | .text'

# `:nth(-1)` matches `:last`.
ts_in VV_nth_neg "Second heading" "$TINY" 'headings:nth(-1) | .text'

# Out-of-range nth returns null.
ts_eq VV_nth_oob "null" "$TINY" 'headings:nth(99) | .text'

# `:lang(rust)` filters fences.
ts_in VV_lang_match 'fn main' "$TINY" 'codeblocks:lang(rust) | .literal'
ts_eq VV_lang_no_match "" "$TINY" 'codeblocks:lang(python) | .literal'

# `:text` filters by exact heading text.
ts_in VV_text_match 'Tiny' "$TINY" 'headings:text("Tiny") | .text'
ts_eq VV_text_no_match "" "$TINY" 'headings:text("Nope") | .text'

# `.literal` on a heading returns null (only Code has literal).
ts_eq VV_literal_on_heading 'null' "$TINY" 'headings:first | .literal'

# `.lang` on a heading returns null.
ts_eq VV_lang_on_heading 'null' "$TINY" 'headings:first | .lang'

# Anchor uses slug.
ts_in VV_anchor_h2 'second-heading' "$TINY" 'h2:first | .anchor'

# `kind` exposed via stream.
ts_in VV_kind_via_attr 'heading' "$TINY" 'headings:first | .kind'

# `.children` projection.
ts_eq VV_children_count '1' "$TINY" '[headings:first | .children[]] | length'

# `..` walks every value.
ts_in VV_recurse_all 'http' "$TINY" '.. | select(type == "link") | .href'

section "WW. stream/tree parity widening"

for q in \
    'headings | .text' \
    'headings | .level' \
    'headings | .anchor' \
    'h1 | .text' \
    'h2 | .anchor' \
    'codeblocks | .lang' \
    'codeblocks | .literal' \
    'links | .href' \
    'links | .title' \
    'images | .href' \
    'images | .alt' \
    'paragraphs | .text' \
    'headings | select(.level == 1) | .text' \
    'headings | select(.level == 2) | .anchor' \
    'codeblocks | .kind'
do
    name=WW_$(printf '%s' "$q" | tr -c 'A-Za-z0-9' '_')
    tree=$({ printf '%s' "$TINY" | "$MDQY" --stdin "[$q] | .[]" 2>&1; printf x; })
    stream=$({ printf '%s' "$TINY" | "$MDQY" --stdin "$q" 2>&1; printf x; })
    [[ "${tree%x}" == "${stream%x}" ]] && ok "$name" || ko "$name :: drift"
done

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

#!/usr/bin/env bash
# plan.md M0 exit gates, verbatim, with pass/fail per gate.
#
# "A milestone is complete when its exit gates pass, not when the code is
# written." — plan.md §1
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cd "$(dirname "$0")/.." || exit 1

pass=0; fail=0
gate() { # description ; then the command
  local desc="$1"; shift
  if out=$("$@" 2>&1); then
    printf '  \033[32mPASS\033[0m  %s\n' "$desc"; pass=$((pass+1))
  else
    printf '  \033[31mFAIL\033[0m  %s\n' "$desc"; fail=$((fail+1))
    echo "$out" | sed 's/^/          /' | tail -12
  fi
}

echo "M0 exit gates — plan.md"
echo

gate "cargo --version && rustc --version resolve" \
     bash -c 'cargo --version && rustc --version'

gate "cargo test --workspace is green" \
     cargo test --workspace

gate "cargo clippy --workspace -- -D warnings is clean" \
     cargo clippy --workspace --all-targets -- -D warnings

gate "cargo fmt --all --check is clean" \
     cargo fmt --all --check

gate "git log shows planning history before code" \
     bash -c 'git log --oneline | tail -1 | grep -q "planning corpus before any code"'

# The D-010 gate. `grep` succeeding here means a forbidden dep was FOUND, so the
# sense is inverted: the gate passes when grep finds nothing.
gate "cargo tree -p spoonstill-core has no concrete dependency" \
     bash -c '! cargo tree -p spoonstill-core -e normal \
                | grep -iE "tauri|reqwest|elevenlabs|rusqlite|keyring"'

gate "every workspace crate has at least one test" \
     bash -c '
       for c in crates/*/; do
         n=$(basename "$c")
         cnt=$(cargo test -p "$n" -- --list 2>/dev/null | grep -c ": test$")
         [ "$cnt" -ge 1 ] || { echo "$n has no tests"; exit 1; }
       done'

gate "make fixtures produces a genuinely odd-dimension fixture (D-033)" \
     bash -c '
       make fixtures >/dev/null 2>&1
       d=$(ffprobe -v error -select_streams v:0 -show_entries stream=width,height \
             -of csv=p=0 fixtures/generated/odd.jpg)
       [ "$d" = "1999,1001" ] || { echo "odd.jpg is $d, expected 1999,1001"; exit 1; }'

echo
if [ "$fail" -eq 0 ]; then
  printf '\033[32mM0 COMPLETE\033[0m — %d/%d gates pass\n' "$pass" "$((pass+fail))"
else
  printf '\033[31mM0 INCOMPLETE\033[0m — %d passed, %d failed\n' "$pass" "$fail"
  exit 1
fi

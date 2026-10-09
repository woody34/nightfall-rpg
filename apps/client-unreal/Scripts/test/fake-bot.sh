#!/usr/bin/env bash
# Stand-in for the Nightfall game binary, honouring the bot-runner contract:
#   fake-bot.sh -game -nullrhi -nosound -unattended -BotScenario=<path.nfs> [-SimGroup=<id>]
# exit 0 pass / 1 fail; writes $SIM_SAVED_DIR/<scenario>.{xml,log,nfr}.
# Directives in the scenario file (one per line): `fake result pass|fail|crash|noreport|norecording|diverge`,
# `fake sleep N`, `fake needgroup` (fails unless -SimGroup was passed).
scenario=""; group=""
for a in "$@"; do
  case "$a" in
    -BotScenario=*) scenario="${a#-BotScenario=}" ;;
    -SimGroup=*) group="${a#-SimGroup=}" ;;
  esac
done
[[ -f "$scenario" ]] || { echo "fake-bot: no scenario" >&2; exit 2; }
name="$(basename "$scenario" .nfs)"
out="${SIM_SAVED_DIR:?}"; mkdir -p "$out"
result=pass; sleep_s=0; needgroup=0
while read -r word verb arg _; do
  [[ "$word" == fake ]] || continue
  case "$verb" in result) result="$arg" ;; sleep) sleep_s="$arg" ;; needgroup) needgroup=1 ;; esac
done <"$scenario"
echo "fake-bot $name group=${group:-none}" | tee "$out/$name.log"
sleep "$sleep_s"
[[ "$result" == crash ]] && exit 139
fail=0; [[ "$result" == fail ]] && fail=1
((needgroup)) && [[ -z "$group" ]] && fail=1
if [[ "$result" != noreport ]]; then
  cat >"$out/$name.xml" <<X
<?xml version="1.0"?>
<testsuite name="$name" tests="1" failures="$fail" errors="0"><testcase classname="$name" name="step"/></testsuite>
X
fi
if [[ "$result" != norecording ]]; then
  if [[ "$result" == diverge ]]; then echo DIVERGE >"$out/$name.nfr"; else echo ok >"$out/$name.nfr"; fi
fi
exit "$fail"

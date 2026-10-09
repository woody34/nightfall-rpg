#!/usr/bin/env bash
# Stand-in for the Nightfall game binary, honouring the bot-runner contract:
#   fake-bot.sh -game -nullrhi -nosound -unattended -BotScenario=<path.nfs> [-SimGroup=<id>]
# exit 0 pass / 1 fail; writes $SIM_SAVED_DIR/<scenario>.{xml,log,nfr}.
# Directives: `fake result pass|fail|crash|noreport|norecording|diverge|greenexit|redreport`,
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
result=pass; sleep_s=0; needgroup=0; amount=1
while read -r word verb arg _; do
  [[ "$word" == fake ]] || continue
  case "$verb" in result) result="$arg" ;; sleep) sleep_s="$arg" ;; count) amount="$arg" ;; needgroup) needgroup=1 ;; esac
done <"$scenario"
echo "fake-bot $name group=${group:-none}" | tee "$out/$name.log"
sleep "$sleep_s"
[[ "$result" == crash ]] && exit 139
fail=0
case "$result" in fail|expectation|scenario|ensure|unknown) fail=1 ;; esac
((needgroup)) && [[ -z "$group" ]] && fail=1
report_fail=$fail; [[ "$result" == redreport ]] && report_fail=1
python3 "$(dirname "$0")/fake-artifacts.py" "$out" "$name" "$result" "$report_fail" "$amount"
if [[ "$result" != norecording ]]; then
  printf "%s %s\n" "$result" "$amount" >"$out/$name.nfr"
fi
if [[ "$result" == sharedrecording ]]; then
  printf "pass %s\n" "$((amount + 1))" >"$out/shared.nfr"
fi
[[ "$result" == greenexit ]] && exit 1
exit "$fail"

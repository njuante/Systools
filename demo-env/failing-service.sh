#!/usr/bin/env bash
# Create (or remove) a deliberately-failing system unit so the Services screen
# has something to show. The Services collector reads `systemctl --failed` at the
# system level, so this needs a *system* unit (hence sudo). Fully reversible.
#
#   demo-env/failing-service.sh up      # create + start the failing unit
#   demo-env/failing-service.sh down    # remove it
set -euo pipefail

UNIT="systui-demo-fail.service"
UNIT_PATH="/etc/systemd/system/${UNIT}"

case "${1:-up}" in
  up)
    sudo tee "${UNIT_PATH}" >/dev/null <<'EOF'
[Unit]
Description=SysTUI demo unit that fails on purpose

[Service]
Type=oneshot
ExecStart=/bin/false
EOF
    sudo systemctl daemon-reload
    # Starts and immediately fails; `|| true` so the script keeps going.
    sudo systemctl start "${UNIT}" || true
    echo "Created ${UNIT} (fails on purpose) — it should now appear under Services."
    ;;
  down)
    sudo systemctl reset-failed "${UNIT}" 2>/dev/null || true
    sudo rm -f "${UNIT_PATH}"
    sudo systemctl daemon-reload
    echo "Removed ${UNIT}."
    ;;
  *)
    echo "usage: $0 [up|down]" >&2
    exit 1
    ;;
esac

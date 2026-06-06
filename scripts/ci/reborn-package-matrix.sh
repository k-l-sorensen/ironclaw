#!/usr/bin/env bash
set -euo pipefail

mode="${1:-tests}"

case "${mode}" in
  tests | clippy)
    ;;
  *)
    echo "usage: $0 [tests|clippy]" >&2
    exit 2
    ;;
esac

cargo metadata --no-deps --format-version 1 \
  | jq -c --arg mode "${mode}" '
      def reborn_test_package:
        (.name | startswith("ironclaw_reborn"))
        or (.name | startswith("ironclaw_product"))
        or (.name == "ironclaw_architecture")
        or (.name == "ironclaw_conversations")
        or (.name == "ironclaw_outbound")
        or (.name == "ironclaw_slack_v2_adapter")
        or (.name == "ironclaw_telegram_v2_adapter")
        or (.name == "ironclaw_triggers")
        or (.name == "ironclaw_wasm_product_adapters")
        or (.name | startswith("ironclaw_webui_v2"));

      def reborn_clippy_dependent:
        (.name == "ironclaw")
        or (.name == "ironclaw_event_streams")
        or (.name == "ironclaw_host_runtime");

      def selected:
        if $mode == "clippy" then
          reborn_test_package or reborn_clippy_dependent
        else
          reborn_test_package
        end;

      [
        .packages[]
        | select(selected)
        | .name
      ]
      | unique
    '

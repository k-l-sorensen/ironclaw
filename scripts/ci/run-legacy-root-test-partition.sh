#!/usr/bin/env bash
set -euo pipefail

partition_count="${LEGACY_ROOT_TEST_PARTITIONS:?LEGACY_ROOT_TEST_PARTITIONS must be set}"
partition_index="${LEGACY_ROOT_TEST_PARTITION:?LEGACY_ROOT_TEST_PARTITION must be set}"
feature_flags="${LEGACY_ROOT_TEST_FEATURE_FLAGS:-}"

if ! [[ "${partition_count}" =~ ^[0-9]+$ ]] || [ "${partition_count}" -lt 1 ]; then
  echo "LEGACY_ROOT_TEST_PARTITIONS must be a positive integer; got '${partition_count}'" >&2
  exit 2
fi

partition_count_int=$((10#${partition_count}))

if ! [[ "${partition_index}" =~ ^[0-9]+$ ]]; then
  echo "LEGACY_ROOT_TEST_PARTITION must be an integer in [0, ${partition_count_int}); got '${partition_index}'" >&2
  exit 2
fi

partition_index_int=$((10#${partition_index}))

if [ "${partition_index_int}" -ge "${partition_count_int}" ]; then
  echo "LEGACY_ROOT_TEST_PARTITION must be an integer in [0, ${partition_count}); got '${partition_index}'" >&2
  exit 2
fi

mapfile -t integration_tests < <(
  find tests -maxdepth 1 -type f -name '*.rs' -print \
    | sed -E 's#^tests/##; s#\.rs$##' \
    | LC_ALL=C sort
)

ran_any=false
for index in "${!integration_tests[@]}"; do
  if (( index % partition_count_int != partition_index_int )); then
    continue
  fi

  test_name="${integration_tests[$index]}"
  ran_any=true
  echo "::group::cargo test --test ${test_name}"
  # shellcheck disable=SC2086 # feature_flags intentionally expands to zero or more Cargo args.
  cargo test ${feature_flags} --test "${test_name}" -- --nocapture
  echo "::endgroup::"
done

if [ "${ran_any}" = false ]; then
  echo "No legacy root integration tests assigned to partition ${partition_index_int} of ${partition_count_int}; passing by design"
fi

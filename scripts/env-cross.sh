# shellcheck shell=bash
# Source this file:  source scripts/env-cross.sh
# Sets Android NDK linker vars for aarch64-linux-android builds on macOS/Linux.

_aginxos_env_cross() {
  local ndk api prebuilt clang ar

  if [[ -n "${ANDROID_NDK_HOME:-}" && -d "${ANDROID_NDK_HOME}" ]]; then
    ndk="${ANDROID_NDK_HOME}"
  elif [[ -d "${HOME}/Library/Android/sdk/ndk" ]]; then
    ndk="$(ls -d "${HOME}/Library/Android/sdk/ndk"/* 2>/dev/null | sort -V | tail -1)"
  elif [[ -d "${HOME}/Android/Sdk/ndk" ]]; then
    ndk="$(ls -d "${HOME}/Android/Sdk/ndk"/* 2>/dev/null | sort -V | tail -1)"
  else
    echo "env-cross: ANDROID_NDK_HOME not found (Android target builds will fail)" >&2
    return 0
  fi

  api="${ANDROID_API:-30}"
  prebuilt="$(echo "${ndk}"/toolchains/llvm/prebuilt/*)"
  if [[ ! -d "${prebuilt}" ]]; then
    echo "env-cross: NDK prebuilt toolchain missing under ${ndk}" >&2
    return 1
  fi

  clang="${prebuilt}/bin/aarch64-linux-android${api}-clang"
  ar="${prebuilt}/bin/llvm-ar"
  if [[ ! -x "${clang}" ]]; then
    # Fall back to highest available API clang
    clang="$(ls "${prebuilt}"/bin/aarch64-linux-android*-clang 2>/dev/null | sort -V | tail -1)"
  fi
  if [[ ! -x "${clang}" ]]; then
    echo "env-cross: no aarch64-linux-android*-clang in ${prebuilt}/bin" >&2
    return 1
  fi

  export ANDROID_NDK_HOME="${ndk}"
  export PATH="${prebuilt}/bin:${PATH}"
  export CC_aarch64_linux_android="${clang}"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${clang}"
  export AR_aarch64_linux_android="${ar}"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_AR="${ar}"

  echo "env-cross: NDK=${ndk}"
  echo "env-cross: linker=${clang}"
}

_aginxos_env_cross

#!/usr/bin/env bash
set -euo pipefail

msix_packaging_root=""
destination=""
configuration="MinSizeRel"

usage() {
  cat <<'EOF'
usage: build-makemsix.sh --msix-packaging-root <path> --destination <path> [--configuration <name>]

Builds makemsix from the microsoft/msix-packaging checkout and copies the runtime
binary plus the libmsix shared library into the destination directory.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --msix-packaging-root)
      shift
      msix_packaging_root="${1:-}"
      ;;
    --destination)
      shift
      destination="${1:-}"
      ;;
    --configuration)
      shift
      configuration="${1:-}"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

if [[ -z "$msix_packaging_root" || -z "$destination" ]]; then
  usage >&2
  exit 1
fi

if [[ ! -f "$msix_packaging_root/CMakeLists.txt" ]]; then
  echo "msix-packaging root was not found at $msix_packaging_root" >&2
  exit 1
fi

if ! command -v cmake >/dev/null 2>&1; then
  echo "cmake is required to build makemsix" >&2
  exit 1
fi

build_dir="$msix_packaging_root/.winget-source-builder/build"
mkdir -p "$build_dir"
mkdir -p "$destination"

platform="$(uname -s)"
cmake_args=(
  -S "$msix_packaging_root"
  -B "$build_dir"
  -DCMAKE_BUILD_TYPE="$configuration"
  -DMSIX_PACK=on
  -DUSE_VALIDATION_PARSER=on
  -DMSIX_SAMPLES=off
  -DMSIX_TESTS=off
)

case "$platform" in
  Linux)
    cmake_args+=(
      -DCMAKE_TOOLCHAIN_FILE="$msix_packaging_root/cmake/linux.cmake"
      -DLINUX=on
    )
    ;;
  Darwin)
    cmake_args+=(
      -DCMAKE_TOOLCHAIN_FILE="$msix_packaging_root/cmake/macos.cmake"
      -DMACOS=on
      -DUSE_MSIX_SDK_ZLIB=on
      -DXML_PARSER=xerces
    )
    ;;
  *)
    echo "unsupported host platform for build-makemsix.sh: $platform" >&2
    exit 1
    ;;
esac

cmake "${cmake_args[@]}"
cmake --build "$build_dir" --target makemsix --config "$configuration" --parallel

if [[ -f "$build_dir/bin/makemsix" ]]; then
  cp "$build_dir/bin/makemsix" "$destination/makemsix"
else
  echo "makemsix build output was not found at $build_dir/bin/makemsix" >&2
  exit 1
fi

find "$build_dir/lib" -maxdepth 1 \( -name 'libmsix*.so*' -o -name 'libmsix*.dylib*' \) -type f -exec cp {} "$destination" \;

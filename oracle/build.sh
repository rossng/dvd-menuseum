#!/bin/sh
# Build the libdvdnav oracle from the reference sources in .ref/libdvdnav.
#
# libdvdnav is compiled from source (not linked from the system) so the oracle
# can read its internal VM state and so that rand() and gettimeofday() can be
# swapped for the deterministic versions the Rust engine also uses.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
src="$root/.ref/libdvdnav"
out="$root/target/oracle"
[ -d "$src/src" ] || { echo "missing $src (fetch libdvdnav sources into .ref/)" >&2; exit 1; }
mkdir -p "$out/gen/dvdnav"

cat > "$out/gen/config.h" <<CFG
#define HAVE_DLFCN_H 1
#define HAVE_INTTYPES_H 1
#define HAVE_STDINT_H 1
#define HAVE_STDLIB_H 1
#define HAVE_STRINGS_H 1
#define HAVE_STRING_H 1
#define HAVE_SYS_STAT_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_UNISTD_H 1
#define HAVE_GETTIMEOFDAY 1
#define VERSION "7.0.1-oracle"
CFG
sed -e 's/@DVDNAV_MAJOR@/7/g' -e 's/@DVDNAV_MINOR@/0/g' -e 's/@DVDNAV_SUB@/1/g' \
    "$src/src/dvdnav/version.h.in" > "$out/gen/dvdnav/version.h"

cd "$out"
# shellcheck disable=SC2046
cc -O1 -g -std=gnu99 -DHAVE_CONFIG_H -DNDEBUG -D_DEFAULT_SOURCE \
   -Drand=oracle_rand -Dgettimeofday=oracle_gettimeofday \
   -I"$out/gen" -I"$src/src" -I"$src/src/vm" $(pkg-config --cflags dvdread) \
   -w -c "$src"/src/*.c "$src"/src/vm/*.c
cc -O1 -g -std=gnu99 -DHAVE_CONFIG_H -I"$out/gen" -I"$src/src" -I"$src/src/vm" $(pkg-config --cflags dvdread) \
   -o "$out/oracle" "$root/oracle/oracle.c" "$out"/*.o $(pkg-config --libs dvdread) -lpthread
echo "$out/oracle"

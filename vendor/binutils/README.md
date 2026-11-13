# GNU binutils for the Rust build (Windows)

Start.bat compiles the server with Rust's `x86_64-pc-windows-gnu` toolchain.
Linking any program needs import libraries that rustc makes with `dlltool`,
and `dlltool` runs the GNU assembler (`as`). Rust's toolchain ships a
`dlltool.exe` but no `as.exe`, so on a PC without MinGW the build fails with

    error: dlltool could not create import library ... dlltool.exe: CreateProcess

These files are GNU binutils (`as`, `dlltool`) and the DLLs they need, taken
unchanged from the WinLibs MinGW-w64 build:

- Archive: winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64msvcrt-14.0.0-r3.zip
  (https://github.com/brechtsanders/winlibs_mingw/releases/tag/16.2.0posix-23.1.3-14.0.0-msvcrt-r3)
  SHA-256 62af32ea58acdea1318ea5dd8eb7017eaee694dd40b269d52a59c769dcfefb82
- Files from its `mingw64/bin` folder; `x86_64-w64-mingw32-as.exe` is a copy of `as.exe`.

GNU binutils is free software under the GNU General Public License v3; its
source code is at https://sourceware.org/binutils/ and the WinLibs build
scripts at https://github.com/brechtsanders/winlibs_mingw. zlib, zstd,
libiconv and gettext (libintl) are under their own free licences.

Start.bat copies this folder to `.rmr\tools\dlltool` and puts it first on
PATH while it builds; nothing is installed on the PC.

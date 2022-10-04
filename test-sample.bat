@echo off
setlocal

rem Run one sample file at given options
rem reads input from samples\%1
rem creates output in out\%1
rem Provide "x86", "x64" or "arm64" as first param to set target.

if not exist out mkdir out

if "x%1"=="xx86" (
    set arch=x86
    set hostarch=x86
    set target=i686-pc-windows-msvc
    shift
) else if "x%1"=="xx64" (
    set arch=x64
    set hostarch=x64
    set target=x86_64-pc-windows-msvc
    shift
) else if "x%1"=="xarm64" (
    set arch=arm64
    set hostarch=x86
    set target=aarch64-pc-windows-msvc
    shift
) else (
    set arch=x64
    set hostarch=x64
    set target=x86_64-pc-windows-msvc
)

call VsDevCmd -arch=%arch% -host_arch=%hostarch%

set x=%1
shift
cd samples
cargo run --release --target=%target% --example mtpng -- %1 %2 %3 %4 %5 %6 %7 %8 %9 %x% ../out/%x%
cd ..

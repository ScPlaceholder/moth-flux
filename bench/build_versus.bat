@echo off
rem Build the C arms with MSVC /O2 — the highest standard MSVC optimisation level (no /O3 exists).
rem No /arch flag on purpose: the Rust side runs default target features (no AVX, no POPCNT),
rem so the C side gets the same baseline ISA. See bench/versus.c header.
call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cd /d "%~dp0.."
cl /nologo /O2 /W4 bench\versus.c /Fe:target\versus_c.exe /Fo:target\versus_c.obj

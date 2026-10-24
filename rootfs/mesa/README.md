# mesa-nibos: Panfrost only, no LLVM

Arch Linux ARM's `extra/mesa` (1:26.2.4-1) cut down to what the RG DS+ uses (the RK3568's Mali-G52, Panfrost),
so GL starts fast at boot. Arch's libgallium is 54 MB and pulls in libLLVM (171 MB); loading
them cold from the SD card cost ~1.9 s of DSlot's start. This one is 21 MB with no LLVM and no
SPIRV-Tools (`ldd` shows neither). The package is 3.5 MB.

```bash
# in the build machine (aarch64 Arch Linux ARM), from the repository: ~2 min on 13 cores;
# the sources and the build go to ~/mesa-build/nibos, the packages to rootfs/mesa/out/
orb -m rgbuild bash -c 'mkdir -p ~/mesa-build/nibos && cp rootfs/mesa/PKGBUILD ~/mesa-build/nibos/ && cd ~/mesa-build/nibos && PKGDEST="$OLDPWD/rootfs/mesa/out" makepkg -s'
# on the device (asks to remove Arch's mesa: answer y)
sudo pacman -U mesa-nibos-1:26.2.4-2-aarch64.pkg.tar.xz
# back to Arch's
sudo pacman -S mesa
```

What changed against Arch's PKGBUILD:

- One package, `mesa-nibos`. It provides `mesa`, `mesa-libgl` and `opengl-driver` and
  conflicts with `mesa`. It is named differently so `pacman -Syu` won't swap Arch's back in.
  When Arch moves to a newer Mesa, rebuild this at that version.
- `-D gallium-drivers=panfrost`, `-D vulkan-drivers=panfrost` (PanVK, split into its own
  package, `vulkan-panfrost-nibos`, which nothing installs by default: on the G52 it is not
  conformant, needs `PAN_I_WANT_A_BROKEN_VULKAN_DRIVER=1`, and is ~5× slower than GL,
  measured in a 3D emulator), no Vulkan layers, `-D llvm=disabled`,
  `-D spirv-tools=disabled`. Also off: rusticl, VA-API/video codecs, teflon, valgrind, the
  HTML docs. GBM, EGL, GLES2/3 and GLX (through libglvnd) stay, with the same platforms as
  Arch's (x11, wayland, drm, surfaceless). Like Arch's, it has no GLES1.
- Two meson builds. Panfrost's internal shaders (libpan) are OpenCL C, compiled at build time by
  `mesa_clc`, which needs clang, LLVM and libclc. The first build makes just `mesa_clc` and
  `vtn_bindgen2` with LLVM. The second builds the driver with `-D mesa-clc=system`, which
  takes them from `PATH`, so nothing in the package links LLVM.
- `VERSION` is `26.2.4-nibos1.1` (Arch's is `-arch1.1`), so it shows in
  `GL_VERSION` and in libgallium's file name.

Checked on the device: EGL 1.5 on GBM, `Mali-G52 r1 MC1 (Panfrost)`, OpenGL ES 3.1. DSlot's
two-window GLES test (KMSDRM, `SDL_KMSDRM_ATOMIC=0`) runs at 56 fps, as before. `llvm-libs` and
`spirv-tools` stay installed (nothing needs them now; `pacman -Qtdq` lists them).

Most of the 21 MB that's left is the GLSL/NIR compiler and Panfrost's code for every Mali
generation (v4–v13), plus one ~5 MB read-only table. Only the pages that are used get read.

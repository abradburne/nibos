// vblank: time 300 vblanks on each CRTC, the panels' real refresh rate.
//
//   gcc -O2 -o vblank vblank.c $(pkg-config --cflags --libs libdrm)    (in the build machine)
//   vblank                                                             (on the device)
//
// Dynamically linked against libdrm (Arch has no static one), unlike the other tools here.
#include <fcntl.h>
#include <stdio.h>
#include <time.h>
#include <xf86drm.h>
int main(void) {
    int fd = open("/dev/dri/card0", O_RDWR);
    if (fd < 0) { perror("card0"); return 1; }
    for (int crtc = 0; crtc < 2; crtc++) {
        drmVBlank v = {0};
        struct timespec a, b;
        for (int i = 0; i <= 300; i++) {
            v.request.type = DRM_VBLANK_RELATIVE | (crtc ? DRM_VBLANK_SECONDARY : 0);
            v.request.sequence = 1;
            if (drmWaitVBlank(fd, &v)) { perror("vblank"); return 1; }
            if (i == 0) clock_gettime(CLOCK_MONOTONIC, &a);
        }
        clock_gettime(CLOCK_MONOTONIC, &b);
        double s = (b.tv_sec - a.tv_sec) + (b.tv_nsec - a.tv_nsec) / 1e9;
        printf("crtc %d: %.3f Hz\n", crtc, 300 / s);
    }
    return 0;
}

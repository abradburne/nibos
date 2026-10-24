#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
int main(int c, char **v) {
    off_t addr = strtoull(v[1], 0, 0); size_t len = strtoul(v[2], 0, 0);
    int fd = open("/dev/mem", O_RDONLY | O_SYNC); if (fd < 0) { perror("open"); return 1; }
    void *p = mmap(0, len, PROT_READ, MAP_SHARED, fd, addr); if (p == MAP_FAILED) { perror("mmap"); return 1; }
    fwrite(p, 1, len, stdout); return 0;
}

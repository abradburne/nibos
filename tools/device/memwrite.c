/* memwrite ADDR VALUE: write one 32-bit word to a physical address through /dev/mem
 * (page-aligned mmap). For poking SoC registers while debugging. */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
int main(int c, char **v) {
    if (c != 3) { fprintf(stderr, "usage: memwrite ADDR VALUE\n"); return 2; }
    off_t addr = strtoull(v[1], 0, 0); uint32_t val = strtoul(v[2], 0, 0);
    long pg = sysconf(_SC_PAGESIZE); off_t base = addr & ~(off_t)(pg - 1);
    int fd = open("/dev/mem", O_RDWR | O_SYNC); if (fd < 0) { perror("open"); return 1; }
    volatile uint8_t *p = mmap(0, pg, PROT_READ | PROT_WRITE, MAP_SHARED, fd, base);
    if (p == MAP_FAILED) { perror("mmap"); return 1; }
    volatile uint32_t *r = (volatile uint32_t *)(p + (addr - base));
    uint32_t before = *r; *r = val;
    printf("%#llx: %#010x -> wrote %#010x, now %#010x\n", (unsigned long long)addr, before, val, *r);
    return 0;
}

/* Print the arch counter and CLOCK_BOOTTIME: their difference is when the
   kernel started, on the counter. Used by measure.sh. */
#include <stdio.h>
#include <time.h>
#include <stdint.h>
int main(void) {
    uint64_t c, f; struct timespec m, b;
    asm volatile("isb; mrs %0, cntvct_el0" : "=r"(c));
    asm volatile("mrs %0, cntfrq_el0" : "=r"(f));
    clock_gettime(CLOCK_MONOTONIC, &m); clock_gettime(CLOCK_BOOTTIME, &b);
    double cs = (double)c / f, ms = m.tv_sec + m.tv_nsec / 1e9, bs = b.tv_sec + b.tv_nsec / 1e9;
    printf("counter %.6f s  monotonic %.6f  boottime %.6f  counter-boottime %.6f\n", cs, ms, bs, cs - bs);
    return 0;
}

/*
 * nibos-readahead: a systemd generator that reads what the first screen needs into the page
 * cache while systemd is still starting up.
 *
 * Generators run at ~1.4 s from the kernel's start; the unit that draws the first screen starts
 * ~0.7 s later, so its program, its libraries and Mesa can be read in between instead of page by
 * page as they are first touched. The generator forks a reader into its own session and returns
 * at once. The reader runs at idle I/O priority and in 128 KB reads, and only reads the parts of
 * each file a boot actually used: reading whole files (56 MB) took the card from systemd for
 * 0.9 s and started the unit ~0.6 s later.
 *
 * /etc/nibos/readahead.list: "PATH" (whole file) or "PATH OFFSET LENGTH" lines, in the order
 * they are wanted. `nibos-readahead --record LIST` prints, for each path in LIST, the ranges
 * now in the page cache: run it after a boot without read-ahead to make the list.
 *
 * Static, so that it does not itself wait for the dynamic loader and libc to be read.
 */
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#define CHUNK (128 << 10)

static double boottime(void)
{
	struct timespec t;
	clock_gettime(CLOCK_BOOTTIME, &t);
	return t.tv_sec + t.tv_nsec / 1e9;
}

static int record(const char *listfile)
{
	FILE *list = fopen(listfile, "r");
	if (!list)
		return 1;
	long page = sysconf(_SC_PAGESIZE);
	char path[4096];
	while (fgets(path, sizeof path, list)) {
		path[strcspn(path, " \n")] = 0;
		if (path[0] != '/')
			continue;
		int fd = open(path, O_RDONLY);
		struct stat st;
		if (fd < 0 || fstat(fd, &st) < 0 || st.st_size == 0) {
			if (fd >= 0)
				close(fd);
			continue;
		}
		size_t pages = (st.st_size + page - 1) / page;
		void *map = mmap(NULL, st.st_size, PROT_READ, MAP_SHARED, fd, 0);
		unsigned char *vec = malloc(pages);
		if (map != MAP_FAILED && vec && mincore(map, st.st_size, vec) == 0) {
			for (size_t i = 0; i < pages;) {
				if (!(vec[i] & 1)) {
					i++;
					continue;
				}
				size_t j = i;
				while (j < pages && (vec[j] & 1))
					j++;
				printf("%s %zu %zu\n", path, i * page, (j - i) * page);
				i = j;
			}
		}
		free(vec);
		if (map != MAP_FAILED)
			munmap(map, st.st_size);
		close(fd);
	}
	return 0;
}

int main(int argc, char **argv)
{
	if (argc == 3 && strcmp(argv[1], "--record") == 0)
		return record(argv[2]);
	if (fork() != 0)
		return 0;
	setsid();
	/* IOPRIO_WHO_PROCESS, this process, IOPRIO_CLASS_IDLE. */
	syscall(SYS_ioprio_set, 1, 0, 3 << 13);
	setpriority(PRIO_PROCESS, 0, 19);
	FILE *list = fopen("/etc/nibos/readahead.list", "r");
	if (!list)
		return 0;
	double start = boottime();
	static char buf[CHUNK];
	char line[4096 + 64], path[4096] = "";
	int fd = -1;
	long long total = 0;
	while (fgets(line, sizeof line, list)) {
		char p[4096];
		long long off = 0, len = -1;
		if (line[0] != '/' || sscanf(line, "%4095s %lld %lld", p, &off, &len) < 1)
			continue;
		if (strcmp(p, path) != 0) {
			if (fd >= 0)
				close(fd);
			strcpy(path, p);
			fd = open(path, O_RDONLY | O_CLOEXEC);
		}
		if (fd < 0)
			continue;
		for (long long done = 0; len < 0 || done < len;) {
			size_t want = len < 0 || len - done > CHUNK ? CHUNK : (size_t)(len - done);
			ssize_t n = pread(fd, buf, want, off + done);
			if (n <= 0)
				break;
			done += n;
			total += n;
		}
	}
	if (fd >= 0)
		close(fd);
	/* For the boot-time bench: when, and how much. */
	FILE *out = fopen("/run/nibos-readahead", "w");
	if (out) {
		fprintf(out, "%.3f %.3f %lld\n", start, boottime(), total);
		fclose(out);
	}
	return 0;
}

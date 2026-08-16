#define _GNU_SOURCE

#include <errno.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/sysinfo.h>
#include <unistd.h>

static void require_equal(const char *name, long actual, long expected) {
  if (actual != expected) {
    fprintf(stderr, "%s: got %ld, expected %ld\n", name, actual, expected);
    exit(EXIT_FAILURE);
  }
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s EXPECTED_CPU_COUNT\n", argv[0]);
    return EXIT_FAILURE;
  }

  const long expected = strtol(argv[1], NULL, 10);
  if (expected <= 0 || expected > CPU_SETSIZE) {
    fprintf(stderr, "expected CPU count must be in 1..%d\n", CPU_SETSIZE);
    return EXIT_FAILURE;
  }

  cpu_set_t mask;
  memset(&mask, 0xff, sizeof(mask));
  errno = EDOM;
  require_equal("sched_getaffinity", sched_getaffinity(0, sizeof(mask), &mask), 0);
  require_equal("sched_getaffinity errno", errno, EDOM);
  require_equal("sched_getaffinity CPU_COUNT", CPU_COUNT(&mask), expected);

  for (long cpu = 0; cpu < expected; cpu++) {
    require_equal("synthetic affinity low bit", CPU_ISSET(cpu, &mask), 1);
  }
  for (long cpu = expected; cpu < CPU_SETSIZE; cpu++) {
    require_equal("synthetic affinity high bit", CPU_ISSET(cpu, &mask), 0);
  }

  cpu_set_t raw_pid_mask;
  memset(&raw_pid_mask, 0xff, sizeof(raw_pid_mask));
  long kernel_bytes =
      syscall(SYS_sched_getaffinity, getpid(), sizeof(raw_pid_mask), &raw_pid_mask);
  if (kernel_bytes < 0) {
    perror("raw sched_getaffinity");
    return EXIT_FAILURE;
  }
  memset((char *)&raw_pid_mask + kernel_bytes, 0,
         sizeof(raw_pid_mask) - kernel_bytes);

  cpu_set_t public_pid_mask;
  memset(&public_pid_mask, 0xff, sizeof(public_pid_mask));
  errno = EDOM;
  require_equal("nonzero sched_getaffinity",
                sched_getaffinity(getpid(), sizeof(public_pid_mask),
                                  &public_pid_mask),
                0);
  require_equal("nonzero sched_getaffinity errno", errno, EDOM);
  require_equal("nonzero sched_getaffinity passthrough",
                memcmp(&public_pid_mask, &raw_pid_mask, sizeof(public_pid_mask)),
                0);

  errno = EDOM;
  require_equal("sysconf online", sysconf(_SC_NPROCESSORS_ONLN), expected);
  require_equal("sysconf online errno", errno, EDOM);
  errno = EDOM;
  require_equal("sysconf configured", sysconf(_SC_NPROCESSORS_CONF), expected);
  require_equal("sysconf configured errno", errno, EDOM);
  errno = EDOM;
  require_equal("get_nprocs", get_nprocs(), expected);
  require_equal("get_nprocs errno", errno, EDOM);
  errno = EDOM;
  require_equal("get_nprocs_conf", get_nprocs_conf(), expected);
  require_equal("get_nprocs_conf errno", errno, EDOM);

  errno = EDOM;
  if (sysconf(_SC_PAGESIZE) <= 0) {
    fputs("unrelated sysconf query failed\n", stderr);
    return EXIT_FAILURE;
  }
  require_equal("unrelated sysconf errno", errno, EDOM);

  puts("preload probe passed");
  return EXIT_SUCCESS;
}

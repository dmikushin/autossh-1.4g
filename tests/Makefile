# Makefile for autossh unit tests.
#
# Layout:
#   tests/framework.h        - test macros
#   tests/mocks/mocks.{h,c}  - --wrap libc syscalls
#   tests/unit/test_*.c      - one test binary per source file
#
# Each test binary links autossh.c (compiled with UNIT_TEST_NO_MAIN)
# against mocks.c with linker --wrap to redirect libc calls.
#
# Run from the project root:  make -C tests check

CC      ?= gcc
TOPDIR  := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)

# Build with coverage and full warnings; -O0 keeps line numbers stable
CFLAGS  := -g -O0 -Wall -Wextra -Wno-unused-parameter \
           -DUNIT_TEST_NO_MAIN \
           -DVER=\"test\" -DSSH_PATH=\"/usr/bin/ssh\" \
           -I$(TOPDIR) -I$(TOPDIR)/tests \
           --coverage

# Symbols to intercept via ld --wrap
WRAP_SYMS  := kill waitpid sleep alarm time _exit read write getenv poll
WRAP_FLAGS := $(foreach s,$(WRAP_SYMS),-Wl,--wrap=$(s))

LDFLAGS := --coverage
LIBS    := -lnsl

# Discover test sources
UNIT_SRCS := $(wildcard $(TOPDIR)/tests/unit/test_*.c)
UNIT_BINS := $(patsubst $(TOPDIR)/tests/unit/%.c,$(TOPDIR)/tests/unit/%,$(UNIT_SRCS))

# Shared object files
AUTOSSH_TEST_OBJ := $(TOPDIR)/tests/build/autossh-test.o
MOCKS_OBJ        := $(TOPDIR)/tests/build/mocks.o

.PHONY: all check clean

all: $(UNIT_BINS)

# Compile autossh.c with UNIT_TEST_NO_MAIN
$(AUTOSSH_TEST_OBJ): $(TOPDIR)/autossh.c | $(TOPDIR)/tests/build
	$(CC) $(CFLAGS) -c -o $@ $<

# Compile mocks
$(MOCKS_OBJ): $(TOPDIR)/tests/mocks/mocks.c $(TOPDIR)/tests/mocks/mocks.h | $(TOPDIR)/tests/build
	$(CC) $(CFLAGS) -c -o $@ $<

$(TOPDIR)/tests/build:
	mkdir -p $@

# Link rule for each test binary
$(TOPDIR)/tests/unit/%: $(TOPDIR)/tests/unit/%.c $(AUTOSSH_TEST_OBJ) $(MOCKS_OBJ)
	$(CC) $(CFLAGS) $(LDFLAGS) $(WRAP_FLAGS) -o $@ $^ $(LIBS)

check: $(UNIT_BINS)
	@fail=0; pass=0; \
	for t in $(UNIT_BINS); do \
		if "$$t"; then pass=$$((pass+1)); else fail=$$((fail+1)); fi; \
	done; \
	echo ""; \
	echo "==================================="; \
	echo "Unit: $$pass test files passed, $$fail failed"; \
	echo "==================================="; \
	test $$fail -eq 0
	@if [ -x $(TOPDIR)/autossh ]; then \
		chmod +x $(TOPDIR)/tests/integration/*.sh; \
		$(TOPDIR)/tests/integration/run.sh; \
	else \
		echo "Skipping integration: $(TOPDIR)/autossh not built (run 'make' first)"; \
	fi

coverage: check
	@cd $(TOPDIR) && gcov -o tests/build autossh.c >/dev/null 2>&1 || true
	@if [ -f $(TOPDIR)/autossh.c.gcov ]; then \
		echo "Coverage of autossh.c:"; \
		grep -E "^\s+#####:" $(TOPDIR)/autossh.c.gcov | wc -l \
			| awk '{printf "  Uncovered lines: %s\n", $$1}'; \
		grep -cE "^\s+[0-9]+:" $(TOPDIR)/autossh.c.gcov \
			| awk '{printf "  Executed lines:  %s\n", $$1}'; \
	fi

clean:
	rm -rf $(TOPDIR)/tests/build $(UNIT_BINS)
	find $(TOPDIR)/tests -name '*.gcda' -delete -o -name '*.gcno' -delete
	rm -f $(TOPDIR)/*.gcov $(TOPDIR)/*.gcda $(TOPDIR)/*.gcno

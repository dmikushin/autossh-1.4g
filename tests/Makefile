# Makefile for autossh unit tests.
#
# Each test binary links:
#   - the test source (tests/unit/test_*.c)
#   - the mocks library (tests/mocks/mocks.c, with -Wl,--wrap=<libc syscalls>)
#   - the autossh staticlib (target/release/libautossh.a, the entire Rust
#     port; produced by cargo)
#   - a fresh copy of c-shim/errlog.c compiled here for test isolation,
#     supplying the C-side errlog/xerrlog/doerrlog and jumpbuf storage
#
# Run from the project root:  make -C tests check

CC      ?= gcc
TOPDIR  := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)

CFLAGS  := -g -O0 -Wall -Wextra -Wno-unused-parameter \
           -DUNIT_TEST_NO_MAIN \
           -I$(TOPDIR) -I$(TOPDIR)/tests \
           --coverage

WRAP_SYMS  := kill waitpid sleep alarm time _exit read write getenv poll
WRAP_FLAGS := $(foreach s,$(WRAP_SYMS),-Wl,--wrap=$(s))

LDFLAGS := --coverage
LIBS    := -lnsl

UNIT_SRCS := $(wildcard $(TOPDIR)/tests/unit/test_*.c)
UNIT_BINS := $(patsubst $(TOPDIR)/tests/unit/%.c,$(TOPDIR)/tests/unit/%,$(UNIT_SRCS))

CSHIM_OBJ      := $(TOPDIR)/tests/build/cshim.o
MOCKS_OBJ      := $(TOPDIR)/tests/build/mocks.o
RUST_STATICLIB := $(TOPDIR)/target/release/libautossh.a
RUST_LIBS      := -lpthread -ldl -lm

.PHONY: all check clean

all: $(UNIT_BINS)

$(CSHIM_OBJ): $(TOPDIR)/c-shim/errlog.c | $(TOPDIR)/tests/build
	$(CC) $(CFLAGS) -c -o $@ $<

$(MOCKS_OBJ): $(TOPDIR)/tests/mocks/mocks.c $(TOPDIR)/tests/mocks/mocks.h | $(TOPDIR)/tests/build
	$(CC) $(CFLAGS) -c -o $@ $<

$(TOPDIR)/tests/build:
	mkdir -p $@

$(TOPDIR)/tests/unit/%: $(TOPDIR)/tests/unit/%.c $(CSHIM_OBJ) $(MOCKS_OBJ) $(RUST_STATICLIB)
	$(CC) $(CFLAGS) $(LDFLAGS) $(WRAP_FLAGS) -o $@ \
		$(TOPDIR)/tests/unit/$*.c $(CSHIM_OBJ) $(MOCKS_OBJ) \
		$(RUST_STATICLIB) $(LIBS) $(RUST_LIBS)

$(RUST_STATICLIB):
	cd $(TOPDIR) && cargo build --release

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

clean:
	rm -rf $(TOPDIR)/tests/build $(UNIT_BINS)
	find $(TOPDIR)/tests -name '*.gcda' -delete -o -name '*.gcno' -delete
	rm -f $(TOPDIR)/*.gcov $(TOPDIR)/*.gcda $(TOPDIR)/*.gcno

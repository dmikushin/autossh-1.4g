# Convenience Makefile — autossh is a Rust project. The actual
# build lives in Cargo.toml + build.rs; targets here are thin
# wrappers for users with the autoconf habit (`./configure && make
# install`).

VER     := 1.5.1
DESTDIR ?=
prefix  ?= /usr/local
bindir  := $(DESTDIR)$(prefix)/bin
mandir  := $(DESTDIR)$(prefix)/share/man/man1
docdir  := $(DESTDIR)$(prefix)/share/doc/autossh
exdir   := $(DESTDIR)$(prefix)/share/examples/autossh

TARGET  := autossh
CARGOBIN := target/release/$(TARGET)

.PHONY: all check clean install distclean

all: $(TARGET)

$(TARGET): $(CARGOBIN)
	cp $(CARGOBIN) $(TARGET)

$(CARGOBIN): FORCE
	cargo build --release

.PHONY: FORCE
FORCE:

check: $(TARGET)
	$(MAKE) -C tests check

clean:
	-rm -f *.o *.a *.core *~ $(TARGET)
	-$(MAKE) -C tests clean 2>/dev/null || true
	-cargo clean 2>/dev/null || true

distclean: clean

install: $(TARGET)
	mkdir -p -m 755 $(bindir) $(mandir) $(docdir) $(exdir)
	cp $(TARGET) $(bindir)/
	chmod 755 $(bindir)/$(TARGET)
	cp autossh.1 $(mandir)/
	chmod 644 $(mandir)/autossh.1
	cp CHANGES README $(docdir)/
	chmod 644 $(docdir)/CHANGES $(docdir)/README
	cp autossh.host rscreen $(exdir)/
	chmod 644 $(exdir)/autossh.host $(exdir)/rscreen

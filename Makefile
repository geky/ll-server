

.PHONY: build
all build:
	cargo build

.PHONY: run
run:
	./target/debug/ll-server

.PHONY: clean
clean:
	cargo clean

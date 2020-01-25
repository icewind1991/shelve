target/x86_64-unknown-linux-musl/release/shelve:
	cargo b --release --target x86_64-unknown-linux-musl

.PHONY: target/x86_64-unknown-linux-musl/release/shelve

docker: target/x86_64-unknown-linux-musl/release/shelve
	docker build -t icewind1991/shelve .
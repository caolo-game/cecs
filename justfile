build:
    cargo build

test: build
	cargo nextest run --all-features

tsan:
	RUSTFLAGS=-Zsanitizer=thread RUSTDOCFLAGS=-Zsanitizer=thread cargo test --all-features -Zbuild-std  --target x86_64-unknown-linux-gnu

asan:
	RUSTFLAGS=-Zsanitizer=address RUSTDOCFLAGS=-Zsanitizer=address cargo test --all-features -Zbuild-std  --target x86_64-unknown-linux-gnu

sanitize: tsan asan

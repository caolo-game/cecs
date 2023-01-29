tsan:
	RUSTFLAGS=-Zsanitizer=thread RUSTDOCFLAGS=-Zsanitizer=thread cargo nextest run --all-features -Zbuild-std  --target x86_64-unknown-linux-gnu

asan:
	RUSTFLAGS=-Zsanitizer=address RUSTDOCFLAGS=-Zsanitizer=address cargo nextest run --all-features -Zbuild-std  --target x86_64-unknown-linux-gnu

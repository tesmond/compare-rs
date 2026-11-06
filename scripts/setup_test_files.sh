#!/bin/bash
set -e
DIR="/tmp/compare-rs-test"
rm -rf "$DIR"
mkdir -p "$DIR"/{left,right}

# Identical files
cat > "$DIR/left/identical.txt" << 'EOF'
Hello World
This is a test file
Line three
Line four
EOF
cp "$DIR/left/identical.txt" "$DIR/right/identical.txt"

# Different files
cat > "$DIR/left/config.yml" << 'EOF'
server:
  host: localhost
  port: 8080
  timeout: 30
  debug: true
EOF
cat > "$DIR/right/config.yml" << 'EOF'
server:
  host: 0.0.0.0
  port: 8080
  timeout: 60
  log_level: info
  debug: false
EOF

# Files with many changes
cat > "$DIR/left/large.txt" << 'EOF'
alpha
beta
gamma
delta
epsilon
zeta
eta
theta
iota
kappa
EOF
cat > "$DIR/right/large.txt" << 'EOF'
alpha
BETA
gamma
DELTA
epsilon
new-line-here
zeta
eta
theta
IOTA
kappa
lambda
EOF

# Create folders for folder comparison
mkdir -p "$DIR/folders"
mkdir -p "$DIR/folders/left" "$DIR/folders/right"

# Left only
echo "left only content" > "$DIR/folders/left/only_in_left.txt"

# Right only
echo "right only content" > "$DIR/folders/right/only_in_right.txt"

# Different content
echo "version 1" > "$DIR/folders/left/shared.txt"
echo "version 2" > "$DIR/folders/right/shared.txt"

# Identical content
echo "same content" > "$DIR/folders/left/same.txt"
cp "$DIR/folders/left/same.txt" "$DIR/folders/right/same.txt"

# Nested dirs
mkdir -p "$DIR/folders/left/subdir" "$DIR/folders/right/subdir"
echo "nested left" > "$DIR/folders/left/subdir/nested.txt"
echo "nested right" > "$DIR/folders/right/subdir/nested.txt"

# Create a small binary file
printf '\x00\x01\x02\x03\x04\x05\xFF\xFE\xFD' > "$DIR/left/binary.bin"
printf '\x00\x01\x02\x03\x04\x05\xFF\xFE\xFC' > "$DIR/right/binary.bin"

echo "Test files created in $DIR"
echo "left/ : $(ls $DIR/left/ | tr '\n' ' ')"
echo "right/ : $(ls $DIR/right/ | tr '\n' ' ')"
echo "folders/ : $(find $DIR/folders -type f | tr '\n' ' ')"

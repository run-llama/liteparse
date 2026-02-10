#!/bin/bash

# Test script to verify builds are working correctly
# This script tests both the TypeScript build and binary compilation

set -e  # Exit on any error

echo "=== LiteParse Build Test ==="
echo

# Check if Bun is installed
if ! command -v bun &> /dev/null; then
    echo "⚠️  Bun not found. Binary build will be skipped."
    echo "   Install Bun: curl -fsSL https://bun.sh/install | bash"
    echo
    SKIP_BINARY=true
else
    echo "✅ Bun found: $(bun --version)"
    SKIP_BINARY=false
fi

# Check Node.js version
NODE_VERSION=$(node --version)
echo "✅ Node.js: $NODE_VERSION"
echo

# Step 1: Test TypeScript build
echo "Step 1: Testing TypeScript build..."
pnpm build

if [ ! -f "dist/src/lib.js" ]; then
    echo "❌ Library build failed: dist/src/lib.js not found"
    exit 1
fi

if [ ! -f "dist/src/index.js" ]; then
    echo "❌ CLI build failed: dist/src/index.js not found"
    exit 1
fi

if [ ! -f "dist/src/lib.d.ts" ]; then
    echo "❌ TypeScript declarations not generated"
    exit 1
fi

echo "✅ TypeScript build successful"
echo

# Step 2: Test CLI with Node
echo "Step 2: Testing CLI with Node.js..."
node dist/src/index.js --version

if [ $? -eq 0 ]; then
    echo "✅ CLI executable with Node.js works"
else
    echo "❌ CLI failed to run"
    exit 1
fi
echo

# Step 3: Test binary compilation (if Bun is available)
if [ "$SKIP_BINARY" = false ]; then
    echo "Step 3: Testing binary compilation..."
    pnpm build:binary

    if [ ! -f "bin/lp" ] && [ ! -f "bin/lp.exe" ]; then
        echo "❌ Binary build failed: bin/lp not found"
        exit 1
    fi

    echo "✅ Binary compilation successful"
    echo

    # Step 4: Test binary execution
    echo "Step 4: Testing binary execution..."
    if [ -f "bin/lp" ]; then
        ./bin/lp --version
        if [ $? -eq 0 ]; then
            echo "✅ Binary executable works"
        else
            echo "❌ Binary failed to run"
            exit 1
        fi
    elif [ -f "bin/lp.exe" ]; then
        ./bin/lp.exe --version
        if [ $? -eq 0 ]; then
            echo "✅ Binary executable works"
        else
            echo "❌ Binary failed to run"
            exit 1
        fi
    fi
    echo
else
    echo "Step 3: Skipping binary compilation (Bun not installed)"
    echo "Step 4: Skipping binary test"
    echo
fi

# Summary
echo "=== Build Test Summary ==="
echo "✅ TypeScript build: PASSED"
echo "✅ CLI with Node.js: PASSED"

if [ "$SKIP_BINARY" = false ]; then
    echo "✅ Binary compilation: PASSED"
    echo "✅ Binary execution: PASSED"
    echo
    echo "🎉 All tests passed! Your build system is working correctly."
    echo
    echo "Next steps:"
    echo "  • Use 'pnpm parse <file>' to test parsing"
    echo "  • Use './bin/lp parse <file>' to test the binary"
    echo "  • Run 'pnpm build:bun:all' to build for all platforms"
else
    echo "⚠️  Binary build: SKIPPED (install Bun to enable)"
    echo
    echo "✅ TypeScript build is working correctly!"
    echo
    echo "Next steps:"
    echo "  • Install Bun to enable binary compilation: curl -fsSL https://bun.sh/install | bash"
    echo "  • Use 'pnpm parse <file>' to test parsing"
fi

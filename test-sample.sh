# Run one sample file at given options
# reads input from samples/$1
# creates output in out/$1

mkdir -p out && \
cd samples && \
x="$1"
shift
cargo --quiet run --release --example mtpng -- "$@" "$x" "../out/$x" || exit 1
echo ""

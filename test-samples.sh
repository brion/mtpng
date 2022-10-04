# Run all the sample files at given options
# reads input from samples/*.png
# creates output in out/*.png

mkdir -p out && \
cd samples && \
for x in *.png
do
  cargo --quiet run --release --example mtpng -- "$@" "$x" "../out/$x" || exit 1
  echo ""
done

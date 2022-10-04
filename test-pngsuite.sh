# Run all the pngsuite files at given options
# reads input from pngsuite/*.png
# creates output in out/*.png

mkdir -p out && \
cd pngsuite && \
for x in *.png
do
  cargo --quiet run --release --example mtpng -- "$@" "$x" "../out/$x" || exit 1
  echo ""
done

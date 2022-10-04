# Run all the sample files using all the possible filter modes.
# reads input from samples/*.png
# creates output in out/*-(adaptive|..|paeth).png

FILTERS="adaptive none sub up average paeth"
SAMPLES=samples/*.png

for filter in $FILTERS
do
    for xsample in $SAMPLES
    do
        sample=`basename "$xsample"`
        cargo run --quiet --release --example mtpng -- --filter="$filter" "samples/$sample" "out/${sample%.png}-$filter.png"
    done
done

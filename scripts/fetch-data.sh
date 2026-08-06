#!/usr/bin/env bash
# Downloads the benchmark datasets into data/. No account or API key is involved anywhere,
# which is what makes the published numbers reproducible.
#
#   *.isd  NOAA ISD-Lite      hourly weather, tenths
#   *.csv  NOAA CO-OPS        six-minute water level, thousandths
#   *.rdb  USGS NWIS          fifteen-minute river discharge and gage height
set -euo pipefail

YEAR="${YEAR:-2023}"
DEST="${DEST:-data}"
mkdir -p "$DEST"

fetch() {
  local target="$1" url="$2" label="$3"
  if [ -s "$target" ]; then
    echo "have    $label"
    return 0
  fi
  if curl -fsSL --max-time 120 "$url" -o "$target.partial" && [ -s "$target.partial" ]; then
    mv "$target.partial" "$target"
    echo "fetched $label ($(wc -l < "$target") rows)"
  else
    rm -f "$target.partial"
    echo "missing $label" >&2
  fi
}

# Weather stations, chosen to span climates. Names from isd-history.csv.
for station in \
  "080840-99999" `# Logrono/Agoncillo, Spain — continental` \
  "082210-99999" `# Madrid Barajas, Spain — plateau` \
  "071500-99999" `# Le Bourget, France — oceanic` \
  "037720-99999" `# London Heathrow, UK — maritime` \
  "604300-99999" `# Miliana, Algeria — Mediterranean highland` \
  "411940-99999" `# Dubai, UAE — desert` \
  "486980-99999" `# Singapore Changi — equatorial` \
  "722020-12839" `# Miami, USA — humid subtropical` \
  "723650-23050" `# Albuquerque, USA — high desert` \
  "947680-99999" `# Sydney, Australia — southern hemisphere` \
; do
  gz="$DEST/$station-$YEAR.gz"
  target="$DEST/$station-$YEAR.isd"
  if [ ! -s "$target" ]; then
    if curl -fsSL --max-time 120 "https://www.ncei.noaa.gov/pub/data/noaa/isd-lite/$YEAR/$station-$YEAR.gz" -o "$gz"; then
      gunzip -c "$gz" > "$target" && rm -f "$gz"
      echo "fetched isd $station ($(wc -l < "$target") rows)"
    else
      rm -f "$gz"
      echo "missing isd $station" >&2
    fi
  else
    echo "have    isd $station"
  fi
done

# Tide gauges. The API caps water_level requests at 31 days, so one January per gauge.
for gauge in \
  "8724580" `# Key West, Florida` \
  "9414290" `# San Francisco, California` \
  "8443970" `# Boston, Massachusetts` \
  "8518750" `# The Battery, New York` \
; do
  fetch "$DEST/coops-$gauge-$YEAR.csv" \
    "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=water_level&application=graupel&begin_date=${YEAR}0101&end_date=${YEAR}0131&datum=MLLW&station=$gauge&time_zone=gmt&units=metric&format=csv" \
    "coops $gauge"
done

# River gauges, discharge and stage together.
for site in \
  "01646500" `# Potomac at Little Falls, Maryland` \
  "06934500" `# Missouri at Hermann, Missouri` \
  "09380000" `# Colorado at Lees Ferry, Arizona` \
  "14211720" `# Willamette at Portland, Oregon` \
; do
  fetch "$DEST/usgs-$site-$YEAR.rdb" \
    "https://waterservices.usgs.gov/nwis/iv/?format=rdb&sites=$site&startDT=${YEAR}-01-01&endDT=${YEAR}-01-31&parameterCd=00060,00065" \
    "usgs $site"
done

echo
echo "wrote $(ls -1 "$DEST" 2>/dev/null | wc -l) files to $DEST/"

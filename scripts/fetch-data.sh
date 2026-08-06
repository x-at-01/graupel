#!/usr/bin/env bash
# Downloads a year of hourly observations from NOAA's ISD-Lite archive into data/.
# No account or API key is involved, so anyone can reproduce the benchmark.
set -euo pipefail

YEAR="${YEAR:-2023}"
DEST="${DEST:-data}"

# USAF-WBAN identifiers, chosen to span climates so that no single one drives the result.
# Names and coordinates come from https://www.ncei.noaa.gov/pub/data/noaa/isd-history.csv
STATIONS=(
  "080840-99999"  # Logrono/Agoncillo, Spain — continental, 363 m
  "082210-99999"  # Adolfo Suarez Madrid Barajas, Spain — continental plateau, 610 m
  "071500-99999"  # Le Bourget, France — oceanic
  "037720-99999"  # London Heathrow, United Kingdom — maritime
  "604300-99999"  # Miliana, Algeria — Mediterranean highland, 721 m
  "411940-99999"  # Dubai Intl, United Arab Emirates — desert
  "486980-99999"  # Singapore Changi Intl, Singapore — equatorial, almost no annual range
  "722020-12839"  # Miami International Airport, United States — humid subtropical
  "723650-23050"  # Albuquerque Intl Sunport, United States — high desert, 1618 m
  "947680-99999"  # Sydney Observatory Hill, Australia — southern hemisphere
)

mkdir -p "$DEST"

for station in "${STATIONS[@]}"; do
  target="$DEST/${station}-${YEAR}.txt"
  if [ -s "$target" ]; then
    echo "have    ${station}"
    continue
  fi
  url="https://www.ncei.noaa.gov/pub/data/noaa/isd-lite/${YEAR}/${station}-${YEAR}.gz"
  if curl -fsS --max-time 120 "$url" | gunzip -c > "$target.partial"; then
    mv "$target.partial" "$target"
    echo "fetched ${station} ($(wc -l < "$target") rows)"
  else
    rm -f "$target.partial"
    echo "missing ${station}: no data published for ${YEAR}" >&2
  fi
done

echo
echo "wrote $(ls -1 "$DEST"/*.txt 2>/dev/null | wc -l) files to $DEST/"

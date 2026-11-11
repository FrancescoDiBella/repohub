#!/usr/bin/env bash
# Installs Ubuntu packages on a GitHub runner, riding out a mirror outage: apt retries each
# download, and if the runner's Azure mirror still fails, the official archive is used instead.
set -uo pipefail
install() {
  sudo apt-get -o Acquire::Retries=3 update -q && sudo apt-get -o Acquire::Retries=3 install -y -q "$@"
}
install "$@" && exit 0
echo "::warning::The runner's apt mirror failed; using archive.ubuntu.com instead."
sudo sed -i 's|http://azure.archive.ubuntu.com/ubuntu/\?|http://archive.ubuntu.com/ubuntu/|g' /etc/apt/apt-mirrors.txt 2>/dev/null || true
sudo find /etc/apt -name '*.list' -o -name '*.sources' | xargs -r sudo sed -i 's|azure.archive.ubuntu.com|archive.ubuntu.com|g'
sudo sed -i 's|mirror+file:/etc/apt/apt-mirrors.txt|http://archive.ubuntu.com/ubuntu/|g' /etc/apt/sources.list 2>/dev/null || true
sudo find /etc/apt/sources.list.d -name '*.sources' | xargs -r sudo sed -i 's|mirror+file:/etc/apt/apt-mirrors.txt|http://archive.ubuntu.com/ubuntu/|g'
for attempt in 1 2 3; do
  install "$@" && exit 0
  sleep $((attempt * 20))
done
exit 1

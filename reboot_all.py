import requests
import subprocess
import time
import os

peers = [
    # Stockholm
    {
        "number": "13",
        "ip": "13.51.56.252",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Paris
    {
        "number": "12",
        "ip": "52.47.154.248",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Zurich
    {
        "number": "11",
        "ip": "16.62.65.183",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Montreal
    {
        "number": "10",
        "ip": "3.99.221.224",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Cali
    {
        "number": "9",
        "ip": "54.215.31.233",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # London
    {
        "number": "8",
        "ip": "3.8.94.18",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Frankfurt
    {
        "number": "7",
        "ip": "3.120.206.238",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Oregon
    {
        "number": "6",
        "ip": "54.188.124.179",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Ohio
    {
        "number": "5",
        "ip": "18.221.207.66",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Cali
    {
        "number": "4",
        "ip": "54.183.129.110",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Singapore
    {
        "number": "3",
        "ip": "13.212.154.248",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Sydney
    {
        "number": "2",
        "ip": "13.54.192.50",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
    # Seoul
    {
        "number": "1",
        "ip": "13.209.9.52",
        "web_server_port": "56790",
        "libp2p_port": "56789",
        "key_file": "aws_global",
        "id": "",
        "remote_peers_addresses": "",
    },
]

for peer in peers:
    print("\nEmpty docker on peer ", peer["number"])
    cmd = f'ssh -i ./keys/{peer["key_file"]} -t -q ubuntu@{peer["ip"]} \'docker stop $(docker ps -a -q)\''
    process = subprocess.Popen(cmd, shell=True)
    process.wait()
    cmd = f'ssh -i ./keys/{peer["key_file"]} -t -q ubuntu@{peer["ip"]} \'docker rm -vf $(docker ps -aq)\''
    process = subprocess.Popen(cmd, shell=True)
    process.wait()
    cmd = f'ssh -i ./keys/{peer["key_file"]} -t -q ubuntu@{peer["ip"]} \'docker rmi -f $(docker images -aq)\''
    process = subprocess.Popen(cmd, shell=True)
    process.wait()
    print("\nReboot peer ", peer["number"])
    cmd = f'ssh -i ./keys/{peer["key_file"]} -t -q ubuntu@{peer["ip"]} \'sudo reboot\''
    process = subprocess.Popen(cmd, shell=True)
    process.wait()
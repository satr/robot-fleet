#!/bin/sh
set -eu

kafka-topics.sh --bootstrap-server kafka:9092 --create --if-not-exists --topic robot-telemetry --partitions 1 --replication-factor 1
kafka-topics.sh --bootstrap-server kafka:9092 --create --if-not-exists --topic robot-state-events --partitions 1 --replication-factor 1
kafka-topics.sh --bootstrap-server kafka:9092 --create --if-not-exists --topic robot-command-events --partitions 1 --replication-factor 1
kafka-topics.sh --bootstrap-server kafka:9092 --create --if-not-exists --topic robot-emergency-events --partitions 1 --replication-factor 1

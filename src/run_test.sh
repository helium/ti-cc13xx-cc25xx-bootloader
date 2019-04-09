#!/bin/bash

# Helper script for compiling locally and deploying to some target

if [ "$#" -lt 1 ]
then
    echo "must supply IP of remote target (arg 1), and optional -v flag (arg 2)"
    exit
fi

TEST_VAR=$2


set -e
cargo +1.23.0 test --no-run --target=armv7-unknown-linux-gnueabihf

RECENT_FILE=$(perl -e 'print( ( sort { -M $a <=> -M $b } @ARGV )[0], "\n");' ../../../target/armv7-unknown-linux-gnueabihf/debug/cc131x*'')
if [ ${RECENT_FILE: -2} == ".d" ]
then 
RECENT_FILE=${RECENT_FILE::-2}
fi

LATEST_BINARY=`grep '***END' $RECENT_FILE`

EXEC=`echo $LATEST_BINARY | grep -o -P '(?<=debug/).*(?= matches)'`
echo "Running ${EXEC}"

# remove previous test if it is there
ssh -i ~/.ssh/id_rsa debian@$1 "rm ~/${EXEC}" || true

scp -i ~/.ssh/id_rsa ../../../target/armv7-unknown-linux-gnueabihf/debug/$EXEC debian@$1:~/ && \
    ssh -i ~/.ssh/id_rsa debian@$1 "~/${EXEC} ${TEST_VAR}"

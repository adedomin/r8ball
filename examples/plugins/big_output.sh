#!/bin/sh

printf 'PRIVMSG #test :%*s\r\n' 1024 a
printf 'PRIVMSG #test :Hello, World!\r\n'

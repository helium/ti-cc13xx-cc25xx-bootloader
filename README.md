[![Build Status](https://travis-ci.com/helium/ti-cc13xx-bootloader.svg?token=35YrBmyVB8LNrXzjrRop&branch=master)](https://travis-ci.com/helium/ti-cc13xx-bootloader)

# ti-cc13xx-cc26xx-bootloader

This crate implements host-code for [TIs ROM Bootloader (see section 8)](http://www.ti.com/lit/ug/swcu117h/swcu117h.pdf). If certain bits are configured in CCFG, the bootloader may be entered on reboot when the CCFG-indicated pin is pulled in the right direction..

Many things are hard-coded and still need to be abstracted, but this crate has been well-tested. In addition, it currently only support SPI as the physical bootloader interface, but could be support UART with a little bit of work.

This crate will not be maintained or extended by Helium, but is available for forking.

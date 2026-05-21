# blinky

## pll / clock

The `pll.v` file is generated using this:  

```
icepll -i 50 -o 100 -m -f pll.v
```

Note that on the schematic H16 is used for TCXO and H16 is on BANK 1 and the ICETechnicalLibrary states:

> The SB_PLL40_CORE primitive should be used when the source clock of the PLL is driven by FPGA routing i.e. 
> when the PLL source clock originates on the FPGA or is driven by an input pad that is not in the bottom IO 
> bank (IO Bank 2).

and

> The SB_PLL40_PAD primitive should be used when the source clock of the PLL is driven by an input pad that is
> located in the bottom IO bank (IO Bank 2) or the top IO bank (IO Bank 0), and the source clock is not required
> inside the FPGA

so we do NOT use the `-p` argument for `icepll` which says:

> `-p Use SB_PLL40_PAD primitive instead of SB_PLL40_CORE`

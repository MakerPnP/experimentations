module blink_top (
    input  TCXO,      // H16 Bank 1 50 MHz TCXO
    output FPGA_ACT
);

wire clk_100;
wire locked;
wire reset;

assign reset = ~locked;

// ----------------------
// PLL
// ----------------------
pll u_pll (
    .clock_in(TCXO),
    .clock_out(clk_100),
    .locked(locked)
);

// ----------------------
// Application logic
// ----------------------
blink u_blink (
    .clk(clk_100),
    .reset(reset),
    .led(FPGA_ACT)
);

endmodule
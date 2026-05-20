`timescale 1ns/1ps

module blink_tb;

    // Testbench signals
    reg TCXO = 1;
    wire FPGA_ACT;

    // Instantiate your design (DUT = Device Under Test)
    blink #(
        .SPEED(10)   // small number for fast simulation
    ) dut (
        .TCXO(TCXO),
        .FPGA_ACT(FPGA_ACT)
    );

    // Clock generation: 100 MHz simulated clock (10ns period)
    always #5 TCXO = ~TCXO;

    // Simulation control
    initial begin
        $dumpfile("blink.vcd");
        $dumpvars(0, blink_tb);

        // Run simulation for some time
        #500;

        $finish;
    end

endmodule
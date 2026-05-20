module blink
    #(parameter SPEED = 100000)
    (
        input TCXO,
        output FPGA_ACT
    );
    reg rled;
    reg [31:0] counter;
    assign FPGA_ACT = rled;

    initial begin
        counter = 0;
        rled = 0;
    end

    always @(posedge TCXO) begin
        if (counter == SPEED) begin
            rled <= ~rled;
            counter <= 1;
        end else begin
            counter <= counter + 1;
        end
    end
endmodule

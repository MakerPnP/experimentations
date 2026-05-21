module blink
    #(parameter SPEED = 100000)
    (
        input clk,
        input reset,
        output reg led = 0
    );

reg [31:0] counter = 0;

always @(posedge clk) begin
    if (reset) begin
        counter <= 0;
        led <= 0;
    end else begin
        if (counter == SPEED) begin
            led <= ~led;
            counter <= 1;
        end else begin
            counter <= counter + 1;
        end
    end
end

endmodule

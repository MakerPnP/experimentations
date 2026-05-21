
# Olimex-ICE40HX8K-EVB
# yosys+nextpnr

PROJECT  := blink
TOP      := blink
TB       := blink_tb
FAMILY   := ice40
TYPE     := hx8k
PACKAGE  := ct256
VERILOGS := blink.v
TB_VLOGS := blink.v blink_tb.v

FLASH_ALGO_ROOT := ../../../../dev-tools/makerpnpcore-w25q16jv

all: \
	$(PROJECT).bin \
	sim

# -------------------------
# SYNTHESIS (FPGA build)
# -------------------------

$(PROJECT).json: $(VERILOGS)
	yosys -q -l yosys.log -p 'synth_${FAMILY} -top $(TOP) -json $(PROJECT).json' $(VERILOGS)

$(PROJECT).asc: $(PROJECT).json pins.pcf
	nextpnr-${FAMILY} -q -l nextpnr.log --${TYPE} --package ${PACKAGE} --json $(PROJECT).json --pcf pins.pcf --asc $(PROJECT).asc
	@echo ""
	@grep -B 1 "%$$" nextpnr.log
	@echo ""

$(PROJECT).bin: $(PROJECT).asc
	icepack $(PROJECT).asc $(PROJECT).bin


# -------------------------
# SIMULATION (NEW)
# -------------------------

sim:
	iverilog -o $(TB).vvp $(TB_VLOGS)
	vvp $(TB).vvp

wave: sim
	gtkwave blink.vcd

# -------------------------
# CLEAN
# -------------------------

clean:
	rm -rf $(PROJECT).bin $(PROJECT).asc $(PROJECT).json \
		yosys.log nextpnr.log abc.history \
		$(TB).vvp blink.vcd
# -------------------------
# LOAD FPGA
# -------------------------

load: $(PROJECT).bin
	 probe-rs download --chip makerpnpcore-w25q16jv --chip-description-path $(FLASH_ALGO_ROOT)/target/definition.yaml $(PROJECT).bin --binary-format=bin --base-address 0x70000000 --verify
